// Every CUDA declaration kind reachable by the extractor.
__global__ void coverage_kernel() {}

__device__ int coverage_device(int x) { return x * 2; }

__host__ int coverage_host() { return 1; }
