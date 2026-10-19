// Force-included into llama.cpp's HIP sources by build.rs.
//
// llama.cpp's topk-moe.cu (since ggml-org/llama.cpp#29181, 2026-10-17) calls __ballot_sync, but
// ggml's HIP vendor header defines HIP_DISABLE_WARP_SYNC_BUILTINS, so ROCm 6.x does not declare
// it and the HIP build fails. With a full mask the two are the same operation. ROCm 7 declares
// __ballot_sync itself, and the macro would break its declaration.
#pragma once
#include <hip/hip_version.h>
#if HIP_VERSION_MAJOR < 7
#define __ballot_sync(mask, predicate) __ballot(predicate)
#endif
