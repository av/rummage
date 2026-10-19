// Force-included into llama.cpp's HIP sources by build.rs.
//
// llama.cpp's topk-moe.cu (since ggml-org/llama.cpp#29181, 2026-10-17) calls __ballot_sync, but
// ggml's HIP vendor header defines HIP_DISABLE_WARP_SYNC_BUILTINS, so ROCm 6.x does not declare
// it and the HIP build fails. With a full mask the two are the same operation.
#pragma once
#define __ballot_sync(mask, predicate) __ballot(predicate)
