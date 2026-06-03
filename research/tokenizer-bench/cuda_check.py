"""M0 acceptance probe: confirm PyTorch sees the GPU.

    python cuda_check.py     # expect "cuda available: True" in the venv
"""

import torch

print(f"torch {torch.__version__}")
print(f"cuda available: {torch.cuda.is_available()}")
if torch.cuda.is_available():
    print(f"device: {torch.cuda.get_device_name(0)}")
    print(f"capability: {torch.cuda.get_device_capability(0)}")
else:
    print("device: CPU only — install a CUDA build of torch (see requirements.txt)")
