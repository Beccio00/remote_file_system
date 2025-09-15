from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import FileResponse
from pydantic import BaseModel
from pathlib import Path
import shutil
import os

app = FastAPI()

BASE_DIR = Path("./storage")
BASE_DIR.mkdir(exist_ok=True)

class RemoteEntry(BaseModel):
    name: str
    is_dir: bool
    size: int

@app.get("/list/{subpath:path}")
def list_dir(subpath: str):
    target = (BASE_DIR / subpath).resolve()
    if not target.exists() or not target.is_dir():
        raise HTTPException(status_code=404, detail="Directory not found")

    entries = []
    for entry in target.iterdir():
        entries.append(
            RemoteEntry(
                name=entry.name,
                is_dir=entry.is_dir(),
                size=entry.stat().st_size,
            )
        )
    return entries

@app.get("/files/{subpath:path}")
def read_file(subpath: str):
    target = (BASE_DIR / subpath).resolve()
    if not target.exists() or not target.is_file():
        raise HTTPException(status_code=404, detail="File not found")
    return FileResponse(target)

@app.put("/files/{subpath:path}")
async def write_file(subpath: str, request: Request):
    target = (BASE_DIR / subpath).resolve()
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        body = await request.body()
        with open(target, "wb") as f:
            f.write(body)
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Write error: {e}")
    return {"status": "ok"}

@app.post("/mkdir/{subpath:path}")
def create_dir(subpath: str):
    target = (BASE_DIR / subpath).resolve()
    try:
        target.mkdir(parents=True, exist_ok=True)
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Create dir error: {e}")
    return {"status": "ok"}


@app.delete("/files/{subpath:path}")
def delete_path(subpath: str):
    target = (BASE_DIR / subpath).resolve()
    if not target.exists():
        raise HTTPException(status_code=404, detail="Path not found")
    try:
        if target.is_file():
            target.unlink()
        else:
            shutil.rmtree(target)
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Delete error: {e}")
    return {"status": "ok"}