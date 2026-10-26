#!/usr/bin/env python3
"""Run the binary over a directory of PDFs; resumable. usage: eval-corpus.py PDFDIR OUTDIR [budget_seconds]"""
import sys, os, json, subprocess, time, glob
pdfdir, out = sys.argv[1], sys.argv[2]
budget = float(sys.argv[3]) if len(sys.argv) > 3 else 500
os.makedirs(out, exist_ok=True)
binp = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "pdf-ocr")
from pypdf import PdfReader
t0 = time.time()
log = os.path.join(out, "results.jsonl")
done = set()
if os.path.exists(log):
    done = {json.loads(l)["file"] for l in open(log)}
files = sorted(glob.glob(os.path.join(pdfdir, "**", "*.pdf"), recursive=True))
from concurrent.futures import ThreadPoolExecutor
import threading
lock = threading.Lock()
JOBS = int(os.environ.get("JOBS", "1"))

def one(f):
    rel = os.path.relpath(f, pdfdir)
    if rel in done or time.time() - t0 > budget: return
    try: pages = len(PdfReader(f).pages)
    except Exception: pages = None
    s = time.time()
    try:
        p = subprocess.run([binp], input=json.dumps({"path": os.path.abspath(f)}).encode(), capture_output=True, timeout=600)
        rc, so, se = p.returncode, p.stdout, p.stderr.decode(errors="replace").strip()
    except subprocess.TimeoutExpired:
        rc, so, se = -9, b"", "timeout 600s"
    dt = time.time() - s
    kind = None
    if rc == 0:
        j = json.loads(so); kind = j["kind"]
        open(os.path.join(out, rel.replace("/", "__") + ".json"), "wb").write(so)
        if kind == "derived":
            open(os.path.join(out, rel.replace("/", "__") + ".md"), "w").write(j["fragments"]["text"])
    with lock:
        open(log, "a").write(json.dumps({"file": rel, "pages": pages, "rc": rc, "kind": kind, "secs": round(dt, 2), "err": se, "bytes": os.path.getsize(f)}) + "\n")

with ThreadPoolExecutor(JOBS) as ex:
    list(ex.map(one, files))
