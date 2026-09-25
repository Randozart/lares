"""Lares frozen-vision microservice.

Hosts the small frozen models that offload the VLM: OWLv2 for
open-vocabulary detection (landmarks and task spaces, dark-proof), CLIP
for zero-shot room classification, and RelateAnything for relation
triplets between boxes. One process, one GPU, lazy model loading.

Runs on port 8877 beside lares-server; see lares-vision.service.
"""

import base64
import io
import os
import threading

import numpy as np
import torch
from fastapi import FastAPI, HTTPException
from PIL import Image, ImageOps
from pydantic import BaseModel, Field

app = FastAPI(title="lares-vision", version="0.1.0")

DEVICE = os.environ.get("LARES_VISION_DEVICE", "cuda" if torch.cuda.is_available() else "cpu")
MAX_SIDE = 1280

# Loaded on first use, guarded by a lock; each maps to a torch module.
_models: dict = {}
_model_lock = threading.Lock()


def _load(name: str, builder):
    """Load a model once; subsequent calls return the cached instance."""
    with _model_lock:
        if name not in _models:
            _models[name] = builder()
        return _models[name]


def _decode(image_b64: str) -> Image.Image:
    """Decode a base64 JPEG, honor EXIF rotation, cap the long side."""
    try:
        raw = base64.b64decode(image_b64, validate=True)
        image = ImageOps.exif_transpose(Image.open(io.BytesIO(raw))).convert("RGB")
    except Exception as exc:
        raise HTTPException(status_code=400, detail=f"bad image: {exc}") from exc
    if max(image.size) > MAX_SIDE:
        image.thumbnail((MAX_SIDE, MAX_SIDE))
    return image


class DetectRequest(BaseModel):
    """Open-vocabulary detection over one frame."""

    image_b64: str
    queries: list[str]
    threshold: float = 0.3


class DetectItem(BaseModel):
    """One detection, box in Lares convention [ymin,xmin,ymax,xmax] 0-1000."""

    label: str
    score: float
    box: list[int] = Field(min_length=4, max_length=4)


class DetectResponse(BaseModel):
    """Detections for the requested queries."""

    device: str
    detections: list[DetectItem]


class RelateRequest(BaseModel):
    """Relation scoring between supplied boxes on one frame."""

    image_b64: str
    labels: list[str]
    boxes: list[list[int]] = Field(min_length=2)
    vocabulary: list[str] | None = None
    topk: int = 20


class RelateResponse(BaseModel):
    """Ranked relation triplets as (subject, predicate, object, score)."""

    device: str
    triplets: list[dict]


class CalibRequest(BaseModel):
    """Dark-blob detection for boresight calibration frames."""

    image_b64: str
    dark_threshold: int = 80
    min_area: int = 3
    max_area: int = 400


class Blob(BaseModel):
    """One dark blob: centroid + metrics."""

    x: float
    y: float
    area: int
    w: int
    h: int
    fill: float


class CalibResponse(BaseModel):
    """Detected blobs, largest first, plus decoded frame size."""

    blobs: list[Blob]
    frame_w: int
    frame_h: int


class RoomRequest(BaseModel):
    """Zero-shot room classification over one frame."""

    image_b64: str
    rooms: list[str] = Field(min_length=2)


class RoomResponse(BaseModel):
    """Top room label with its softmax score."""

    device: str
    room: str
    confidence: float
    scores: dict[str, float]


@app.get("/health")
def health() -> dict:
    """Liveness plus which device and models are loaded."""
    return {"status": "ok", "device": DEVICE, "loaded": sorted(_models)}


def _build_owl():
    """Create the OWLv2 detector and processor on the service device."""
    from transformers import Owlv2ForObjectDetection, Owlv2Processor

    processor = Owlv2Processor.from_pretrained("google/owlv2-base-patch16-ensemble")
    model = Owlv2ForObjectDetection.from_pretrained(
        "google/owlv2-base-patch16-ensemble", dtype=torch.float32,
    ).to(DEVICE)
    model.eval()
    return processor, model


def _detect(req: DetectRequest) -> DetectResponse:
    """Score open-vocabulary queries against one frame."""
    if not req.queries:
        raise HTTPException(status_code=400, detail="queries must not be empty")
    processor, model = _load("owlv2", _build_owl)
    image = _decode(req.image_b64)
    inputs = processor(text=[req.queries], images=image, return_tensors="pt").to(DEVICE)
    with torch.no_grad():
        outputs = model(**inputs)
    results = processor.post_process_grounded_object_detection(
        outputs,
        target_sizes=torch.tensor([image.size[::-1]]).to(DEVICE),
        threshold=req.threshold,
    )[0]
    width, height = image.size
    items = []
    for score, label, box in zip(
        results["scores"].tolist(), results["labels"].tolist(), results["boxes"].tolist(),
    ):
        xmin, ymin, xmax, ymax = box
        items.append(DetectItem(
            label=req.queries[label],
            score=round(score, 4),
            box=[
                round(ymin / height * 1000), round(xmin / width * 1000),
                round(ymax / height * 1000), round(xmax / width * 1000),
            ],
        ))
    items.sort(key=lambda item: item.score, reverse=True)
    return DetectResponse(device=DEVICE, detections=items)


def _build_clip():
    """Create the CLIP model and processor for room classification."""
    from transformers import CLIPModel, CLIPProcessor

    model = CLIPModel.from_pretrained("openai/clip-vit-base-patch32").to(DEVICE)
    processor = CLIPProcessor.from_pretrained("openai/clip-vit-base-patch32")
    model.eval()
    return model, processor


def _room(req: RoomRequest) -> RoomResponse:
    """Classify the frame against candidate room captions."""
    model, processor = _load("clip", _build_clip)
    image = _decode(req.image_b64)
    captions = [f"a photo of a {room}" for room in req.rooms]
    inputs = processor(text=captions, images=image, return_tensors="pt", padding=True).to(DEVICE)
    with torch.no_grad():
        logits = model(**inputs).logits_per_image.softmax(dim=-1)[0]
    scores = {room: round(float(p), 4) for room, p in zip(req.rooms, logits.tolist())}
    best = max(scores.items(), key=lambda kv: kv[1])[0]
    return RoomResponse(device=DEVICE, room=best, confidence=scores[best], scores=scores)


def _build_relsgg():
    """Create the frozen relation model."""
    from relsgg import RelateAnything

    model = RelateAnything.from_pretrained("maelic/relsgg-vits16plus", device=DEVICE)
    return model


def _relate(req: RelateRequest) -> RelateResponse:
    """Score relations between the supplied labeled boxes."""
    if len(req.labels) != len(req.boxes):
        raise HTTPException(status_code=400, detail="labels and boxes must align")
    model = _load("relsgg", _build_relsgg)
    if req.vocabulary:
        model.set_vocabulary(req.vocabulary)
    image = _decode(req.image_b64)
    width, height = image.size
    boxes_px = []
    for ymin, xmin, ymax, xmax in req.boxes:
        boxes_px.append([
            int(xmin / 1000 * width), int(ymin / 1000 * height),
            int(xmax / 1000 * width), int(ymax / 1000 * height),
        ])
    triplets = model.predict(image, boxes_px, topk=req.topk)
    out = []
    for t in triplets:
        sub = t.subject_idx
        obj = t.object_idx
        out.append({
            "subject": req.labels[sub] if sub < len(req.labels) else f"obj{sub}",
            "predicate": str(t.predicate),
            "object": req.labels[obj] if obj < len(req.labels) else f"obj{obj}",
            "score": round(float(t.score), 4),
        })
    return RelateResponse(device=DEVICE, triplets=out)


def _dark_blobs(
    arr, dark_threshold: int, min_area: int, max_area: int
) -> list[Blob]:
    """Label dark connected components; return round-ish blobs."""
    height, width = arr.shape
    dark = arr < dark_threshold
    visited = np.zeros_like(dark, dtype=bool)
    blobs = []
    for y0, x0 in zip(*np.nonzero(dark)):
        if visited[y0, x0]:
            continue
        stack = [(y0, x0)]
        visited[y0, x0] = True
        pixels = []
        while stack:
            y, x = stack.pop()
            pixels.append((y, x))
            for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                ny, nx = y + dy, x + dx
                if 0 <= ny < height and 0 <= nx < width and dark[ny, nx] and not visited[ny, nx]:
                    visited[ny, nx] = True
                    stack.append((ny, nx))
        if not (min_area <= len(pixels) <= max_area):
            continue
        ys = [p[0] for p in pixels]
        xs = [p[1] for p in pixels]
        w = max(xs) - min(xs) + 1
        h = max(ys) - min(ys) + 1
        if w > 3 * h or h > 3 * w:
            continue
        fill = len(pixels) / (w * h)
        if fill < 0.5:
            continue
        blobs.append(Blob(
            x=round(sum(xs) / len(xs), 1),
            y=round(sum(ys) / len(ys), 1),
            area=len(pixels),
            w=w,
            h=h,
            fill=round(fill, 2),
        ))
    blobs.sort(key=lambda b: b.area, reverse=True)
    return blobs[:10]


def _calib_pips(req: CalibRequest) -> CalibResponse:
    """Find dark calibration-marker blobs on one frame."""
    image = _decode(req.image_b64).convert("L")
    arr = np.asarray(image)
    return CalibResponse(
        blobs=_dark_blobs(arr, req.dark_threshold, req.min_area, req.max_area),
        frame_w=image.width,
        frame_h=image.height,
    )


app.add_api_route("/detect", _detect, methods=["POST"], response_model=DetectResponse)
app.add_api_route("/calib/pips", _calib_pips, methods=["POST"], response_model=CalibResponse)
app.add_api_route("/room", _room, methods=["POST"], response_model=RoomResponse)
app.add_api_route("/relate", _relate, methods=["POST"], response_model=RelateResponse)
