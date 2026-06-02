//! Deterministic document-type detection — the B5 runtime gate.
//!
//! Phase 1's semantic ontology (ADR-0002: `Character`/`Place`/`Event`/…) is
//! **narrative-specific**. Force-fitting a code-heavy README or a present-tense
//! technical manual into that ontology produces noise, not signal. So before the
//! semantic layers run, we classify the document from cheap **structural and
//! lexical** features and recommend which extraction pipeline actually fits.
//!
//! This is pure, deterministic, native-free code — the same text always yields
//! the same [`Classification`] (pinned by [`tests::classify_is_deterministic`]),
//! so it lives here in `doctree-core` next to the structure walker rather than
//! with the gated semantic code. No model is consulted: narrative-vs-not is a
//! coarse decision the surface features answer well, and keeping it deterministic
//! means the gate is verified headlessly by the default `cargo test`. (An
//! optional LLM *confirmation* for low-confidence cases is a deliberate deferral
//! — see ADR-0005.)
//!
//! The signals (all in `[0, 1]`, computed over a single fixed pass):
//!
//! * `structure_ratio` — share of *words* that sit on heading / list / code /
//!   table lines (word-weighted, so prose line-wrapping doesn't skew it). High ⇒
//!   reference material, not prose.
//! * `dialogue_ratio` — share of non-blank lines carrying a double-quote. High ⇒
//!   fiction dialogue.
//! * `pronoun_ratio` — first/third-person personal pronouns per word. High ⇒
//!   character-driven prose. (Second-person `you` is excluded: it skews
//!   instructional, not narrative.)
//! * `past_tense_ratio` — `-ed` verbs plus common irregular past verbs per word.
//!   High ⇒ recounted action, the spine of narration.

use serde::{Deserialize, Serialize};

/// What kind of document this is — selects which extraction pipeline fits.
/// Serializes to a stable snake_case tag for the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentClass {
    /// Prose fiction: dialogue, characters, recounted (past-tense) action.
    /// Phase 1's narrative ontology fits — run the full hybrid.
    Narrative,
    /// Non-narrative prose: essays, articles, manuals, documentation. The
    /// narrative ontology does not fit, but ontology-agnostic similarity does.
    Expository,
    /// Heading / list / code / table-dominated reference material with little
    /// flowing prose. The semantic layers add noise; keep to the spine.
    Structured,
    /// Too little text to classify with any confidence.
    Unknown,
}

/// Which extraction layers suit a document of a given [`DocumentClass`]. This is
/// the *ideal* recommendation from the document type alone, independent of which
/// native features a given build actually compiled in (that reconciliation
/// happens in the command layer). Serializes to a snake_case tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedPipeline {
    /// Spine + LLM narrative semantics + embedding similarity (narrative).
    NarrativeHybrid,
    /// Spine + embedding similarity, but **not** the narrative LLM ontology
    /// (expository prose: concepts relate, but there are no characters/events).
    StructuralPlusSimilarity,
    /// Deterministic spine only (structured/reference material, or unknown).
    StructuralOnly,
}

/// The surface features the classifier measured, all normalized to `[0, 1]`
/// (except `word_count`). Surfaced to the frontend (camelCase) so the routing
/// decision is explainable, and exposed for tests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationSignals {
    /// Total word tokens in the document.
    pub word_count: usize,
    /// Share of words sitting on markdown/structural lines (word-weighted).
    pub structure_ratio: f32,
    /// Share of non-blank lines containing a double-quote (dialogue proxy).
    pub dialogue_ratio: f32,
    /// First/third-person personal pronouns as a share of all words.
    pub pronoun_ratio: f32,
    /// Past-tense verbs (`-ed` + common irregulars) as a share of all words.
    pub past_tense_ratio: f32,
}

/// The classification verdict: the [`DocumentClass`], a `[0, 1]` confidence, and
/// the [`ClassificationSignals`] that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub class: DocumentClass,
    /// How strongly the signals support `class`, in `[0, 1]`.
    pub confidence: f32,
    pub signals: ClassificationSignals,
}

impl Classification {
    /// The extraction pipeline that suits this document's class.
    pub fn recommended_pipeline(&self) -> RecommendedPipeline {
        match self.class {
            DocumentClass::Narrative => RecommendedPipeline::NarrativeHybrid,
            DocumentClass::Expository => RecommendedPipeline::StructuralPlusSimilarity,
            DocumentClass::Structured | DocumentClass::Unknown => {
                RecommendedPipeline::StructuralOnly
            }
        }
    }

    /// Would an LLM *confirmation* of this verdict be worthwhile? (The deferred
    /// ADR-0005 step, finally wired under the `llm` feature in Phase 6 #6.)
    ///
    /// Only the **prose boundary** is worth a model's opinion: `Narrative` vs
    /// `Expository` is the genuinely fuzzy call the surface features can land on
    /// the wrong side of when the signals are weak. The other verdicts are *not*
    /// asked of a model:
    /// * `Structured` is decided by `structure_ratio` — a high-precision, purely
    ///   structural fact a language model can't improve on.
    /// * `Unknown` means there isn't enough text to judge; a model can't conjure
    ///   signal that isn't there.
    ///
    /// So this is true exactly when the class is `Narrative`/`Expository` *and*
    /// the confidence sits below [`CONFIRM_BELOW_CONFIDENCE`] (the ambiguous
    /// band around the 0.5 narrative-score boundary). Pure; native-free — the
    /// gate decides *whether* to consult a model without consulting one.
    pub fn warrants_confirmation(&self) -> bool {
        matches!(
            self.class,
            DocumentClass::Narrative | DocumentClass::Expository
        ) && self.confidence < CONFIRM_BELOW_CONFIDENCE
    }

    /// Fold a model's confirmation verdict back into this classification. The
    /// model is authoritative for the **class** (that's what we asked it to
    /// adjudicate), but the measured [`ClassificationSignals`] are facts and are
    /// kept untouched; only `class` and `confidence` change:
    /// * **Model agrees** with the deterministic class → two independent methods
    ///   concur, so confidence is raised (blended toward 1, never below the prior
    ///   value): `0.5 * (1 + prior)`.
    /// * **Model overrides** the deterministic class → adopt the model's class at
    ///   a moderate [`CONFIRMED_FLOOR`] "model-decided" confidence.
    ///
    /// Pure and total; native-free (the *caller* runs the model, this just
    /// reconciles the answer).
    pub fn with_confirmed_class(self, confirmed: DocumentClass) -> Classification {
        let confidence = if confirmed == self.class {
            clamp01(0.5 * (1.0 + self.confidence))
        } else {
            CONFIRMED_FLOOR
        };
        Classification {
            class: confirmed,
            confidence,
            signals: self.signals,
        }
    }
}

// --- thresholds (named for testability; tuned for short fixtures + real docs) --

/// Below this many words there isn't enough signal to classify.
const MIN_WORDS: usize = 12;
/// A `Narrative`/`Expository` verdict below this confidence sits in the ambiguous
/// band near the narrative-score boundary, where an optional LLM confirmation is
/// worthwhile (see [`Classification::warrants_confirmation`]). Public so the
/// command layer and tests share the one definition.
pub const CONFIRM_BELOW_CONFIDENCE: f32 = 0.65;
/// The confidence assigned when a model **overrides** the deterministic class:
/// moderate — the model is now the authority, but a single confirmation isn't
/// certainty (see [`Classification::with_confirmed_class`]).
pub const CONFIRMED_FLOOR: f32 = 0.7;
/// At/above this share of structural lines the document is reference material.
const STRUCTURE_DOMINATES: f32 = 0.5;
/// Narrative score at/above this is classified [`DocumentClass::Narrative`].
const NARRATIVE_THRESHOLD: f32 = 0.5;

// Per-signal saturation points: the value at which a signal contributes its full
// weight to the narrative score. Picked from typical fiction densities.
const DIALOGUE_FULL: f32 = 0.15;
const PRONOUN_FULL: f32 = 0.06;
const PAST_FULL: f32 = 0.08;

// Weights of each component in the blended narrative score (sum to 1).
const W_DIALOGUE: f32 = 0.40;
const W_PRONOUN: f32 = 0.35;
const W_PAST: f32 = 0.25;

/// First- and third-person personal pronouns. Second-person (`you`/`your`) is
/// intentionally omitted — it signals instructions, not narration.
const NARRATIVE_PRONOUNS: &[&str] = &[
    "i", "me", "my", "mine", "we", "us", "our", "ours", "he", "him", "his", "she", "her", "hers",
    "they", "them", "their", "theirs", "himself", "herself", "themselves",
];

/// Common irregular past-tense verbs that don't end in `-ed`, so the `-ed`
/// heuristic alone would miss the backbone of narration.
const IRREGULAR_PAST: &[&str] = &[
    "was", "were", "had", "said", "went", "came", "saw", "told", "knew", "thought", "took",
    "found", "felt", "looked", "turned", "made", "gave", "got", "began", "ran", "held", "stood",
    "sat", "spoke", "heard", "kept", "left", "met", "put", "read", "set", "wept", "drew", "fell",
    "rose", "broke", "wore", "bore", "lay", "led", "lost", "won", "sent", "spent", "built",
];

/// Classify a document from its structural and lexical surface features.
/// Deterministic and native-free.
pub fn classify_document(text: &str) -> Classification {
    let signals = measure(text);

    // Too little text to judge.
    if signals.word_count < MIN_WORDS {
        // Confidence rises toward 0.5 as we approach the floor (still "unknown",
        // just less starkly so); an empty doc is maximally unknown.
        let conf = 0.5 + 0.5 * (1.0 - signals.word_count as f32 / MIN_WORDS as f32);
        return Classification {
            class: DocumentClass::Unknown,
            confidence: clamp01(conf),
            signals,
        };
    }

    // Reference material: mostly headings/lists/code/tables rather than prose.
    if signals.structure_ratio >= STRUCTURE_DOMINATES {
        return Classification {
            class: DocumentClass::Structured,
            confidence: clamp01(signals.structure_ratio),
            signals,
        };
    }

    // Otherwise: how strongly does it read as narrative?
    let score = narrative_score(&signals);
    if score >= NARRATIVE_THRESHOLD {
        Classification {
            class: DocumentClass::Narrative,
            confidence: clamp01(score),
            signals,
        }
    } else {
        // Non-narrative prose. Confidence is how clearly it *isn't* narrative.
        Classification {
            class: DocumentClass::Expository,
            confidence: clamp01(1.0 - score),
            signals,
        }
    }
}

/// Blend the per-signal saturations into a single `[0, 1]` narrative score.
fn narrative_score(s: &ClassificationSignals) -> f32 {
    let dialogue = (s.dialogue_ratio / DIALOGUE_FULL).min(1.0);
    let pronoun = (s.pronoun_ratio / PRONOUN_FULL).min(1.0);
    let past = (s.past_tense_ratio / PAST_FULL).min(1.0);
    W_DIALOGUE * dialogue + W_PRONOUN * pronoun + W_PAST * past
}

/// Single-pass measurement of every signal.
fn measure(text: &str) -> ClassificationSignals {
    // Line scan: count words living on structural lines (word-weighted so a lone
    // heading over wrapped prose stays low), and lines carrying dialogue.
    let mut non_blank = 0usize;
    let mut quoted = 0usize;
    let mut structural_words = 0usize;
    let mut in_code_fence = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        non_blank += 1;

        let is_structural = if is_code_fence(line) {
            // The fence delimiter and everything inside the block is structure.
            in_code_fence = !in_code_fence;
            true
        } else {
            in_code_fence || is_structural_line(line)
        };
        if is_structural {
            structural_words += words(line).count();
        }

        if line.contains('"') || line.contains('\u{201C}') || line.contains('\u{201D}') {
            quoted += 1;
        }
    }

    // Word scan: total words plus the lexical narrative signals.
    let mut word_count = 0usize;
    let mut pronouns = 0usize;
    let mut past = 0usize;
    for word in words(text) {
        word_count += 1;
        if NARRATIVE_PRONOUNS.contains(&word.as_str()) {
            pronouns += 1;
        }
        if is_past_tense(&word) {
            past += 1;
        }
    }

    let ratio = |num: usize, den: usize| if den == 0 { 0.0 } else { num as f32 / den as f32 };
    ClassificationSignals {
        word_count,
        structure_ratio: ratio(structural_words, word_count),
        dialogue_ratio: ratio(quoted, non_blank),
        pronoun_ratio: ratio(pronouns, word_count),
        past_tense_ratio: ratio(past, word_count),
    }
}

/// A markdown code fence (```` ``` ```` or `~~~`), possibly with a language tag.
fn is_code_fence(line: &str) -> bool {
    line.starts_with("```") || line.starts_with("~~~")
}

/// Is this (trimmed, non-fence) line a structural marker rather than prose?
fn is_structural_line(line: &str) -> bool {
    // Markdown heading.
    if line.starts_with('#') {
        return true;
    }
    // Table row: starts and ends with a pipe, or is a `---|---` separator.
    if line.starts_with('|') && line.ends_with('|') && line.len() > 1 {
        return true;
    }
    // Unordered list bullet: `- `, `* `, `+ ` (the marker must be followed by a
    // space so an em-dash sentence or `*emphasis*` isn't miscounted).
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        if rest.starts_with(' ') {
            return true;
        }
    }
    // Ordered list item: `1. ` / `1) `.
    if is_ordered_list_item(line) {
        return true;
    }
    false
}

/// `12. text` / `3) text` — leading digits then `.`/`)` then a space.
fn is_ordered_list_item(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i + 1 >= bytes.len() {
        return false;
    }
    (bytes[i] == b'.' || bytes[i] == b')') && bytes[i + 1] == b' '
}

/// Lowercased alphabetic word tokens (apostrophes kept inside, e.g. `don't`).
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter_map(|raw| {
            let w = raw.trim_matches('\'').to_lowercase();
            if w.chars().any(|c| c.is_alphabetic()) {
                Some(w)
            } else {
                None
            }
        })
}

/// A past-tense verb heuristic: a common irregular past, or an `-ed` word long
/// enough to be a verb rather than a short adjective.
fn is_past_tense(word: &str) -> bool {
    if IRREGULAR_PAST.contains(&word) {
        return true;
    }
    word.len() >= 4 && word.ends_with("ed")
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NARRATIVE: &str = "Mara found the letter on the table where Vane had left it. \
        She read it twice, then looked out at the dark harbor. \
        \"The ship is gone,\" she said quietly. He turned and watched her, but said nothing. \
        They had waited for weeks, and now the cove was empty and the storm was coming in.";

    const EXPOSITORY: &str = "The function parses the input string and returns a token stream. \
        Each token is validated against the schema before it is emitted to the consumer. \
        Memory allocation is amortized so the parser runs in linear time on typical inputs. \
        Configuration options control the buffer size and the maximum nesting depth allowed.";

    const STRUCTURED: &str = "# Installation\n\n- Install the toolchain\n- Clone the repository\n\
        - Run the build\n\n## Usage\n\n1. Open the file\n2. Select a command\n\n```\ncargo build\n\
        cargo test\n```\n\n| Flag | Effect |\n|------|--------|\n| --release | optimized build |";

    #[test]
    fn classify_is_deterministic() {
        assert_eq!(classify_document(NARRATIVE), classify_document(NARRATIVE));
        assert_eq!(classify_document(STRUCTURED), classify_document(STRUCTURED));
    }

    #[test]
    fn narrative_prose_classifies_as_narrative() {
        let c = classify_document(NARRATIVE);
        assert_eq!(c.class, DocumentClass::Narrative, "signals: {:?}", c.signals);
        assert!(c.confidence >= NARRATIVE_THRESHOLD);
        assert_eq!(c.recommended_pipeline(), RecommendedPipeline::NarrativeHybrid);
    }

    #[test]
    fn technical_prose_classifies_as_expository() {
        let c = classify_document(EXPOSITORY);
        assert_eq!(c.class, DocumentClass::Expository, "signals: {:?}", c.signals);
        assert_eq!(
            c.recommended_pipeline(),
            RecommendedPipeline::StructuralPlusSimilarity
        );
    }

    #[test]
    fn heading_list_code_doc_classifies_as_structured() {
        let c = classify_document(STRUCTURED);
        assert_eq!(c.class, DocumentClass::Structured, "signals: {:?}", c.signals);
        assert!(c.signals.structure_ratio >= STRUCTURE_DOMINATES);
        assert_eq!(c.recommended_pipeline(), RecommendedPipeline::StructuralOnly);
    }

    #[test]
    fn tiny_or_empty_doc_is_unknown() {
        assert_eq!(classify_document("").class, DocumentClass::Unknown);
        assert_eq!(classify_document("Hello there.").class, DocumentClass::Unknown);
        // Unknown routes to the safe structural-only path.
        assert_eq!(
            classify_document("").recommended_pipeline(),
            RecommendedPipeline::StructuralOnly
        );
    }

    #[test]
    fn a_single_heading_does_not_make_prose_structured() {
        // A chapter heading over a block of narration must NOT trip Structured:
        // one structural line out of many prose lines keeps structure_ratio low.
        let doc = format!("# Chapter One\n\n{NARRATIVE}");
        let c = classify_document(&doc);
        assert!(
            c.signals.structure_ratio < STRUCTURE_DOMINATES,
            "one heading over prose stays low: {:?}",
            c.signals
        );
        assert_eq!(c.class, DocumentClass::Narrative);
    }

    #[test]
    fn signals_are_bounded_and_confidence_in_unit_range() {
        for doc in [NARRATIVE, EXPOSITORY, STRUCTURED, "", "short"] {
            let c = classify_document(doc);
            for r in [
                c.signals.structure_ratio,
                c.signals.dialogue_ratio,
                c.signals.pronoun_ratio,
                c.signals.past_tense_ratio,
            ] {
                assert!((0.0..=1.0).contains(&r), "ratio out of range: {r} for {doc:?}");
            }
            assert!((0.0..=1.0).contains(&c.confidence));
        }
    }

    #[test]
    fn dialogue_and_pronouns_are_measured() {
        let s = measure(NARRATIVE);
        assert!(s.dialogue_ratio > 0.0, "narrative has a quoted line");
        assert!(s.pronoun_ratio > 0.0, "she/he/they present");
        assert!(s.past_tense_ratio > 0.0, "found/had/watched present");
    }

    // --- #6: optional LLM confirmation of low-confidence prose verdicts --------

    /// Build a synthetic verdict with a given class + confidence (signals are
    /// irrelevant to the confirmation logic, so they're zeroed).
    fn verdict(class: DocumentClass, confidence: f32) -> Classification {
        Classification {
            class,
            confidence,
            signals: ClassificationSignals {
                word_count: 100,
                structure_ratio: 0.0,
                dialogue_ratio: 0.0,
                pronoun_ratio: 0.0,
                past_tense_ratio: 0.0,
            },
        }
    }

    #[test]
    fn only_low_confidence_prose_warrants_confirmation() {
        // Ambiguous prose (either side of the boundary, below the threshold) → yes.
        assert!(verdict(DocumentClass::Narrative, 0.55).warrants_confirmation());
        assert!(verdict(DocumentClass::Expository, 0.6).warrants_confirmation());
        // Confident prose → no need to ask a model.
        assert!(!verdict(DocumentClass::Narrative, 0.85).warrants_confirmation());
        assert!(!verdict(DocumentClass::Expository, 0.9).warrants_confirmation());
        // Structured / Unknown are never confirmation candidates, even when the
        // confidence is low (structure is a fact; unknown lacks text).
        assert!(!verdict(DocumentClass::Structured, 0.4).warrants_confirmation());
        assert!(!verdict(DocumentClass::Unknown, 0.5).warrants_confirmation());
    }

    #[test]
    fn the_confirmation_threshold_is_the_exact_boundary() {
        // Strictly below warrants; at/above does not (so the constant is the pin).
        let just_below = CONFIRM_BELOW_CONFIDENCE - 0.01;
        let at = CONFIRM_BELOW_CONFIDENCE;
        assert!(verdict(DocumentClass::Narrative, just_below).warrants_confirmation());
        assert!(!verdict(DocumentClass::Narrative, at).warrants_confirmation());
    }

    #[test]
    fn model_agreement_raises_confidence_and_keeps_class_and_signals() {
        let before = verdict(DocumentClass::Narrative, 0.55);
        let after = before.with_confirmed_class(DocumentClass::Narrative);
        assert_eq!(after.class, DocumentClass::Narrative);
        // Blended toward 1: 0.5 * (1 + 0.55) = 0.775; strictly higher than before.
        assert!((after.confidence - 0.775).abs() < 1e-6, "got {}", after.confidence);
        assert!(after.confidence > before.confidence);
        // Signals are facts — untouched by confirmation.
        assert_eq!(after.signals, before.signals);
    }

    #[test]
    fn model_override_adopts_the_new_class_at_the_confirmed_floor() {
        // The deterministic gate said Expository at 0.55; the model says Narrative.
        let before = verdict(DocumentClass::Expository, 0.55);
        let after = before.with_confirmed_class(DocumentClass::Narrative);
        assert_eq!(after.class, DocumentClass::Narrative);
        assert!((after.confidence - CONFIRMED_FLOOR).abs() < 1e-6);
        // The recommendation now follows the corrected class.
        assert_eq!(after.recommended_pipeline(), RecommendedPipeline::NarrativeHybrid);
    }

    #[test]
    fn confirmed_confidence_stays_in_unit_range() {
        for conf in [0.0, 0.3, 0.5, 0.64, 0.99] {
            let agree = verdict(DocumentClass::Narrative, conf)
                .with_confirmed_class(DocumentClass::Narrative);
            let override_ = verdict(DocumentClass::Narrative, conf)
                .with_confirmed_class(DocumentClass::Expository);
            assert!((0.0..=1.0).contains(&agree.confidence));
            assert!((0.0..=1.0).contains(&override_.confidence));
        }
    }
}
