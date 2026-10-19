#!/usr/bin/env python3
"""Make reference vectors for the rummage-embed parity test.

Runs the official google/embeddinggemma-2 model through sentence-transformers in
float32 on CPU and writes reference.json next to this script.

Setup (throwaway venv, kept out of the repo):

    uv venv -p 3.12 /tmp/eg2-ref && . /tmp/eg2-ref/bin/activate
    uv pip install --index-url https://download.pytorch.org/whl/cpu torch torchvision
    uv pip install "sentence-transformers>=6.1.0" transformers pillow soundfile
    HF_HOME=/mnt/models/huggingface python -I make_reference.py

Fixtures (all freely licensed, see fixtures/LICENSES.md):
    images  bread.jpg, cat.jpg, lighthouse.jpg (Wikimedia Commons, CC0)
    audio   speech_*.wav (generated with espeak-ng, 16 kHz mono)
    video   cat_zoom.mp4 (generated from cat.jpg with ffmpeg, 4 s at 10 fps; needs ffmpeg on PATH)
"""

import json
import pathlib
import subprocess
import sys

import numpy as np
import soundfile as sf
import torch
from PIL import Image
from sentence_transformers import SentenceTransformer

HERE = pathlib.Path(__file__).resolve().parent
FIX = HERE / "fixtures"
MODEL = "google/embeddinggemma-2"

QUERY = "task: search result | query: "


def doc(title, text):
    return f"title: {title or 'none'} | text: {text}"


TEXTS = [
    ("q_bread", "query", QUERY + "how do I bake sourdough bread at home?"),
    ("q_cat", "query", QUERY + "a cat sleeping in the sun"),
    (
        "d_bread",
        "document",
        doc(
            "Sourdough basics",
            "Mix flour, water, salt and an active starter. Let the dough ferment overnight, "
            "shape it, then bake it in a preheated Dutch oven at 250 C for about 40 minutes.",
        ),
    ),
    (
        "d_cat",
        "document",
        doc(
            None,
            "Cats sleep for up to sixteen hours a day, and they often pick a warm, sunny "
            "windowsill or porch for their naps.",
        ),
    ),
    (
        "d_tides",
        "document",
        doc(
            "Ocean tides",
            "Tides are caused by the gravitational pull of the moon and the sun on the oceans, "
            "which gives most coasts two high tides every day.",
        ),
    ),
]
IMAGES = [("img_bread", "bread.jpg"), ("img_cat", "cat.jpg"), ("img_lighthouse", "lighthouse.jpg")]
AUDIO = [("aud_bread", "speech_bread.wav"), ("aud_lighthouse", "speech_lighthouse.wav")]
VIDEO = [("vid_cat", "cat_zoom.mp4")]

# Which item each query should be nearest to, within each group.
RETRIEVAL = {
    "q_bread": {"document": "d_bread", "image": "img_bread", "audio": "aud_bread"},
    "q_cat": {"document": "d_cat", "image": "img_cat"},
}


def probe_video(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0", "-count_frames",
         "-show_entries", "stream=width,height,r_frame_rate,nb_read_frames", "-of", "json", path],
        check=True, capture_output=True, text=True).stdout
    s = json.loads(out)["streams"][0]
    num, den = s["r_frame_rate"].split("/")
    return float(num) / float(den), int(s["nb_read_frames"]), int(s["width"]), int(s["height"])


def decode_frames(path, indices, width, height):
    select = "+".join(f"eq(n\\,{i})" for i in indices)
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-vf", f"select='{select}'", "-fps_mode", "passthrough",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        check=True, capture_output=True).stdout
    frames = np.frombuffer(raw, dtype=np.uint8).reshape(len(indices), height, width, 3)
    return frames


def main():
    torch.manual_seed(0)
    model = SentenceTransformer(MODEL, device="cpu", model_kwargs={"dtype": torch.float32})
    for p in model.parameters():
        assert p.dtype == torch.float32, p.dtype
    proc = model[0].processor
    tok = proc.tokenizer

    items = []

    def add(item_id, kind, source, vec, extra=None):
        v = np.asarray(vec, dtype=np.float64)
        assert v.shape == (768,), v.shape
        assert np.isfinite(v).all()
        entry = {
            "id": item_id,
            "kind": kind,
            "source": source,
            "norm": float(np.linalg.norm(v)),
            "vector": [round(float(x), 7) for x in v],
        }
        entry.update(extra or {})
        items.append(entry)

    # Texts: the prefixes are already part of the string, so no prompt_name.
    vecs = model.encode([t for _, _, t in TEXTS], convert_to_numpy=True, normalize_embeddings=True)
    for (item_id, kind, text), v in zip(TEXTS, vecs):
        ids = tok(text)["input_ids"]
        add(item_id, kind, text, v, {"n_tokens": len(ids), "first_ids": ids[:3], "last_ids": ids[-3:]})

    for item_id, name in IMAGES:
        img = Image.open(FIX / name).convert("RGB")
        v = model.encode([img], convert_to_numpy=True, normalize_embeddings=True)[0]
        enc = proc(images=[[img]], return_tensors="pt")
        ids = enc["input_ids"][0].tolist()
        add(item_id, "image", name, v, {"n_tokens": len(ids), "first_ids": ids[:3], "last_ids": ids[-3:],
                                         "width": img.width, "height": img.height})

    for item_id, name in AUDIO:
        samples, sr = sf.read(FIX / name, dtype="float32")
        assert sr == 16000 and samples.ndim == 1
        v = model.encode([{"array": samples, "sampling_rate": sr}], convert_to_numpy=True,
                         normalize_embeddings=True)[0]
        enc = proc(audio=[samples], sampling_rate=sr, return_tensors="pt")
        ids = enc["input_ids"][0].tolist()
        add(item_id, "audio", name, v, {"n_tokens": len(ids), "first_ids": ids[:3], "last_ids": ids[-3:],
                                         "n_samples": int(samples.shape[0])})

    for item_id, name in VIDEO:
        path = str(FIX / name)
        fps, n, width, height = probe_video(path)
        duration = n / fps
        # Same frame sampling as EmbeddingGemma2VideoProcessor.sample_frames (1 fps, at most 32 frames).
        step = fps / 1.0
        idx = [min(n - 1, int(i * step)) for i in range(max(1, int(duration * 1.0)))]
        assert len(idx) <= 32
        # Decode the sampled frames with ffmpeg so the Rust test can get byte-identical frames
        # with the same command (see decode_frames in tests/parity.rs).
        frames = decode_frames(path, idx, width, height)
        video = {"array": frames, "video_metadata": {"fps": fps, "total_num_frames": len(idx),
                                                     "duration": duration}}
        v = model.encode([{"video": video}], convert_to_numpy=True, normalize_embeddings=True,
                         processing_kwargs={"video": {"do_sample_frames": False}})[0]
        add(item_id, "video", name, v, {"frame_indices": idx, "fps": fps, "num_frames": n,
                                         "width": width, "height": height})

    out = {
        "model": MODEL,
        "dtype": "float32",
        "runtime": {
            "torch": torch.__version__,
            "transformers": __import__("transformers").__version__,
            "sentence_transformers": __import__("sentence_transformers").__version__,
        },
        "retrieval": RETRIEVAL,
        "items": items,
    }
    (HERE / "reference.json").write_text(json.dumps(out, indent=1) + "\n")

    # Print a quick similarity table so the fixtures can be eyeballed.
    by_id = {it["id"]: np.array(it["vector"]) for it in items}
    for q, groups in RETRIEVAL.items():
        for kind, want in groups.items():
            cands = [it["id"] for it in items if it["kind"] == kind]
            sims = {c: float(by_id[q] @ by_id[c]) for c in cands}
            best = max(sims, key=sims.get)
            print(f"{q} -> {kind}: best={best} want={want} {sims}", file=sys.stderr)
            assert best == want


if __name__ == "__main__":
    main()
