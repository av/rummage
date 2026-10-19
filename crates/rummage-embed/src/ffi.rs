//! Hand-written bindings for the parts of llama.h, mtmd.h and ggml-backend.h the engine uses.
//! Parameter structs are built by `src/shim.cpp`, so only plain functions and two small
//! structs (`llama_embd`, `mtmd_input_text`) are mirrored here.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};

pub type llama_token = i32;
pub type llama_pos = i32;
pub type llama_seq_id = i32;

#[repr(C)]
pub struct llama_model {
    _p: [u8; 0],
}
#[repr(C)]
pub struct llama_context {
    _p: [u8; 0],
}
#[repr(C)]
pub struct llama_vocab {
    _p: [u8; 0],
}
#[repr(C)]
pub struct llama_batch_ext {
    _p: [u8; 0],
}
#[repr(C)]
pub struct mtmd_context {
    _p: [u8; 0],
}
#[repr(C)]
pub struct mtmd_bitmap {
    _p: [u8; 0],
}
#[repr(C)]
pub struct mtmd_input_chunk {
    _p: [u8; 0],
}
#[repr(C)]
pub struct mtmd_input_chunks {
    _p: [u8; 0],
}
#[repr(C)]
pub struct mtmd_batch {
    _p: [u8; 0],
}

#[repr(C)]
pub struct llama_embd {
    pub data: *const f32,
    pub n_rows: usize,
    pub n_embd: usize,
}

#[repr(C)]
pub struct mtmd_input_text {
    pub text: *const c_char,
    pub text_len: usize,
    pub add_special: bool,
    pub parse_special: bool,
}

pub const LLAMA_PROCESS_TYPE_DECODE: c_int = 1;
pub const LLAMA_POOLING_TYPE_MEAN: c_int = 1;

pub const MTMD_INPUT_CHUNK_TYPE_TEXT: c_int = 0;
pub const MTMD_INPUT_CHUNK_TYPE_IMAGE: c_int = 1;
pub const MTMD_INPUT_CHUNK_TYPE_AUDIO: c_int = 2;

pub const GGML_BACKEND_DEVICE_TYPE_CPU: c_int = 0;

pub type ggml_backend_dev_t = *mut c_void;

unsafe extern "C" {
    // shim.cpp
    pub fn rmg_log_set(min_level: c_int);
    pub fn rmg_backend_init();
    pub fn rmg_model_load(path: *const c_char, n_gpu_layers: i32) -> *mut llama_model;
    pub fn rmg_context_new(
        model: *mut llama_model,
        max_tokens: u32,
        n_seq_max: u32,
        n_threads: i32,
        flash_attn: i32,
        op_offload: bool,
    ) -> *mut llama_context;
    pub fn rmg_mtmd_init(
        mmproj_path: *const c_char,
        model: *const llama_model,
        use_gpu: bool,
        n_threads: i32,
        image_min_tokens: i32,
        image_max_tokens: i32,
        flash_attn: i32,
        batch_max_tokens: i32,
        warmup: bool,
    ) -> *mut mtmd_context;

    // llama.h
    pub fn llama_model_free(model: *mut llama_model);
    pub fn llama_free(ctx: *mut llama_context);
    pub fn llama_model_get_vocab(model: *const llama_model) -> *const llama_vocab;
    pub fn llama_model_n_embd_inp(model: *const llama_model) -> i32;
    pub fn llama_model_n_embd_out(model: *const llama_model) -> i32;
    pub fn llama_model_size(model: *const llama_model) -> u64;
    pub fn llama_model_desc(model: *const llama_model, buf: *mut c_char, buf_size: usize) -> i32;
    pub fn llama_pooling_type(ctx: *const llama_context) -> c_int;
    pub fn llama_n_seq_max(ctx: *const llama_context) -> u32;
    pub fn llama_n_batch(ctx: *const llama_context) -> u32;
    pub fn llama_vocab_bos(vocab: *const llama_vocab) -> llama_token;
    pub fn llama_vocab_eos(vocab: *const llama_vocab) -> llama_token;
    pub fn llama_tokenize(
        vocab: *const llama_vocab,
        text: *const c_char,
        text_len: i32,
        tokens: *mut llama_token,
        n_tokens_max: i32,
        add_special: bool,
        parse_special: bool,
    ) -> i32;
    pub fn llama_get_embeddings_seq(ctx: *mut llama_context, seq_id: llama_seq_id) -> *mut f32;
    pub fn llama_synchronize(ctx: *mut llama_context);
    pub fn llama_print_system_info() -> *const c_char;

    pub fn llama_batch_ext_init(ctx: *mut llama_context) -> *mut llama_batch_ext;
    pub fn llama_batch_ext_free(batch: *mut llama_batch_ext);
    pub fn llama_batch_ext_clear(batch: *mut llama_batch_ext);
    pub fn llama_batch_ext_add_token(
        batch: *mut llama_batch_ext,
        seq_id: llama_seq_id,
        id: llama_token,
    ) -> i32;
    pub fn llama_batch_ext_add_embd(
        batch: *mut llama_batch_ext,
        seq_id: llama_seq_id,
        embd: llama_embd,
    ) -> i32;
    pub fn llama_batch_ext_set_pos(
        batch: *mut llama_batch_ext,
        idx: i32,
        pos: *const llama_pos,
    ) -> bool;
    pub fn llama_batch_ext_set_output_embd(
        batch: *mut llama_batch_ext,
        idx: i32,
        value: bool,
    ) -> bool;
    pub fn llama_process(ctx: *mut llama_context, kind: c_int, batch: *mut llama_batch_ext) -> i32;

    // mtmd.h
    pub fn mtmd_free(ctx: *mut mtmd_context);
    pub fn mtmd_default_marker() -> *const c_char;
    pub fn mtmd_support_vision(ctx: *const mtmd_context) -> bool;
    pub fn mtmd_support_audio(ctx: *const mtmd_context) -> bool;
    pub fn mtmd_get_audio_sample_rate(ctx: *const mtmd_context) -> c_int;
    pub fn mtmd_bitmap_init(nx: u32, ny: u32, data: *const u8) -> *mut mtmd_bitmap;
    pub fn mtmd_bitmap_init_from_audio(n_samples: usize, data: *const f32) -> *mut mtmd_bitmap;
    pub fn mtmd_bitmap_free(bitmap: *mut mtmd_bitmap);
    pub fn mtmd_input_chunks_init() -> *mut mtmd_input_chunks;
    pub fn mtmd_input_chunks_size(chunks: *const mtmd_input_chunks) -> usize;
    pub fn mtmd_input_chunks_get(
        chunks: *const mtmd_input_chunks,
        idx: usize,
    ) -> *const mtmd_input_chunk;
    pub fn mtmd_input_chunks_free(chunks: *mut mtmd_input_chunks);
    pub fn mtmd_input_chunk_get_type(chunk: *const mtmd_input_chunk) -> c_int;
    pub fn mtmd_input_chunk_get_tokens_text(
        chunk: *const mtmd_input_chunk,
        n_tokens_output: *mut usize,
    ) -> *const llama_token;
    pub fn mtmd_input_chunk_get_n_tokens(chunk: *const mtmd_input_chunk) -> usize;
    pub fn mtmd_tokenize(
        ctx: *const mtmd_context,
        output: *mut mtmd_input_chunks,
        text: *const mtmd_input_text,
        bitmaps: *const *const mtmd_bitmap,
        n_bitmaps: usize,
    ) -> i32;
    pub fn mtmd_encode_chunk(ctx: *mut mtmd_context, chunk: *const mtmd_input_chunk) -> i32;
    pub fn mtmd_get_output_embd(ctx: *mut mtmd_context) -> *mut f32;
    pub fn mtmd_batch_init(ctx: *mut mtmd_context) -> *mut mtmd_batch;
    pub fn mtmd_batch_free(batch: *mut mtmd_batch);
    pub fn mtmd_batch_add_chunk(batch: *mut mtmd_batch, chunk: *const mtmd_input_chunk) -> i32;
    pub fn mtmd_batch_encode(batch: *mut mtmd_batch) -> i32;
    pub fn mtmd_batch_get_output_embd(
        batch: *mut mtmd_batch,
        chunk: *const mtmd_input_chunk,
    ) -> *mut f32;

    // ggml-backend.h
    pub fn ggml_backend_dev_count() -> usize;
    pub fn ggml_backend_dev_get(index: usize) -> ggml_backend_dev_t;
    pub fn ggml_backend_dev_name(dev: ggml_backend_dev_t) -> *const c_char;
    pub fn ggml_backend_dev_description(dev: ggml_backend_dev_t) -> *const c_char;
    pub fn ggml_backend_dev_type(dev: ggml_backend_dev_t) -> c_int;
    pub fn ggml_backend_dev_memory(dev: ggml_backend_dev_t, free: *mut usize, total: *mut usize);
}
