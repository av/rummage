// Fills in llama.cpp's parameter structs so the Rust side never has to mirror their layout.
// Everything else is called directly through the C APIs in llama.h and mtmd.h.

#include "ggml.h"
#include "llama.h"
#include "mtmd.h"

#include <atomic>
#include <cstdio>

static std::atomic<int> g_min_level{GGML_LOG_LEVEL_WARN};

static void rmg_log(enum ggml_log_level level, const char * text, void *) {
    // CONT lines continue the previous message; keep them only when that level is shown.
    static thread_local bool last_shown = false;
    bool show;
    if (level == GGML_LOG_LEVEL_CONT) {
        show = last_shown;
    } else {
        show = level >= g_min_level.load();
        last_shown = show;
    }
    if (show) {
        fputs(text, stderr);
    }
}

extern "C" {

// 0 = everything (debug), 2 = info, 3 = warnings (default), 4 = errors only, 5 = silent.
void rmg_log_set(int min_level) {
    g_min_level.store(min_level);
    llama_log_set(rmg_log, nullptr);
    mtmd_log_set(rmg_log, nullptr);
}

void rmg_backend_init(void) {
    llama_backend_init();
}

// n_gpu_layers < 0 puts every layer on the GPU.
llama_model * rmg_model_load(const char * path, int32_t n_gpu_layers) {
    llama_model_params p = llama_model_default_params();
    p.n_gpu_layers = n_gpu_layers;
    return llama_model_load_from_file(path, p);
}

// A memory-less, non-causal embedding context. Every sequence must fit in one micro-batch,
// so n_batch == n_ubatch == max_tokens.
llama_context * rmg_context_new(llama_model * model, uint32_t max_tokens, uint32_t n_seq_max,
                                int32_t n_threads, int32_t flash_attn, bool op_offload) {
    llama_context_params p = llama_context_default_params();
    p.n_ctx           = max_tokens;
    p.n_batch         = max_tokens;
    p.n_ubatch        = max_tokens;
    p.n_seq_max       = n_seq_max;
    p.n_threads       = n_threads;
    p.n_threads_batch = n_threads;
    p.embeddings      = true;
    p.pooling_type    = LLAMA_POOLING_TYPE_MEAN;
    p.attention_type  = LLAMA_ATTENTION_TYPE_NON_CAUSAL;
    p.flash_attn_type = (enum llama_flash_attn_type) flash_attn;
    // The model has no KV cache; if a cache is ever created, keep it in full precision.
    p.type_k          = GGML_TYPE_F32;
    p.type_v          = GGML_TYPE_F32;
    p.kv_unified      = true;
    p.no_perf         = true;
    // false keeps a CPU-only run from shipping large ops to a GPU anyway.
    p.op_offload      = op_offload;
    return llama_init_from_model(model, p);
}

mtmd_context * rmg_mtmd_init(const char * mmproj_path, const llama_model * model, bool use_gpu,
                             int32_t n_threads, int32_t image_min_tokens, int32_t image_max_tokens,
                             int32_t flash_attn, int32_t batch_max_tokens, bool warmup) {
    mtmd_context_params p = mtmd_context_params_default();
    p.use_gpu          = use_gpu;
    p.print_timings    = false;
    p.n_threads        = n_threads;
    p.flash_attn_type  = (enum llama_flash_attn_type) flash_attn;
    p.warmup           = warmup;
    p.image_min_tokens = image_min_tokens;
    p.image_max_tokens = image_max_tokens;
    if (batch_max_tokens > 0) {
        p.batch_max_tokens = batch_max_tokens;
    }
    return mtmd_init_from_file(mmproj_path, model, p);
}

}
