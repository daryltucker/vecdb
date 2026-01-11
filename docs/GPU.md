# GPU Acceleration

## Microsoft ONNX Runtime

This allows for accelerated CPU and/or GPU embedding.

### CUDA Provider

> You can offload Embedding to the GPU (optional)

#### Grab the ONNX Version
```bash
$ vecdb --version
vecdb v0.0.9
ONNX v1.23.2
```

#### Enable GPU Offloading
```bash
$ cat ~/.config/vecdb/config.toml
...
local_use_gpu = true
concurrency=1
gpu_concurrency=1
...

```

#### Obtain & Install Library Files

```bash
cd /tmp/
#export ORT_VER="1.23.2"
#export ORT_VER="1.23.2"
export ORT_VER="$(vecdb --version | grep ONNX | cut -d' ' -f2 | tr -d 'v')"

# Official Microsoft ONNX Runtime URL
wget https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/onnxruntime-linux-x64-gpu-${ORT_VER}.tgz

tar -xvf onnxruntime-linux-x64-gpu-${ORT_VER}.tgz
cd onnxruntime-linux-x64-gpu-${ORT_VER}/lib

sudo cp libonnxruntime_providers_cuda.so /usr/local/lib/
sudo cp libonnxruntime_providers_shared.so /usr/local/lib/
sudo cp libonnxruntime.so.${ORT_VER} /usr/local/lib/
sudo ln -sf /usr/local/lib/libonnxruntime.so.${ORT_VER} /usr/local/lib/libonnxruntime.so

sudo ldconfig
```

## Performance Characteristics

During ingestion, you may observe a period of high CPU and RAM usage before the GPU starts processing. This is expected behavior due to the architecture of the ingestion pipeline:

1.  **Discovery Phase**: The system first scans the entire target directory to count files and build a processing queue. This is a CPU-intensive serial operation.
2.  **Memory Loading**: To ensure high-quality parsing (via `vecq`), files smaller than 50MB are loaded entirely into memory as strings. If you are processing a directory with many medium-sized files, RAM usage will spike during this load window. 
3.  **Tokenization Latency**: Before the GPU can compute embeddings, the texts must be converted into numerical tokens. This "Tokenization" phase runs on the CPU and processes the entire batch (default 20 chunks) before handing the tensors to the GPU.

> [!TIP]
> If your system has many CPU cores but a smaller GPU, you may want to increase `concurrency` while keeping `gpu_concurrency` at 1 to balance the load.