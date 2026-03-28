from fastapi import FastAPI, HTTPException, Request, Header
from fastapi.responses import FileResponse, Response
from pydantic import BaseModel
from pathlib import Path
import shutil
import os
import uvicorn
from dotenv import load_dotenv

# Runtime configuration loaded from environment variables.
HOST = os.getenv("HOST", "127.0.0.1")
PORT = int(os.getenv("PORT", 8000))
BASE_DIR = Path(os.getenv("BASE_DIR", "./storage"))
DEBUG = os.getenv("DEBUG", "false").lower() == "true"

app = FastAPI()

BASE_DIR = Path("./storage")
BASE_DIR.mkdir(exist_ok=True)


# Directory entry returned to clients for /list responses.
class RemoteEntry(BaseModel):
    name: str
    is_dir: bool
    size: int

# GET /list/{subpath}: returns direct children metadata for a directory.
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

# GET /files/{subpath}: downloads a file; supports HTTP Range for partial reads.
@app.get("/files/{subpath:path}")
def read_file(subpath: str, range: str = Header(None)):
    target = (BASE_DIR / subpath).resolve()
    if not target.exists() or not target.is_file():
        raise HTTPException(status_code=404, detail="File not found")

    file_size = target.stat().st_size

    # Handles partial reads so large files can be streamed efficiently.
    if range and range.startswith("bytes="):
        range_spec = range[6:]
        start_str, end_str = range_spec.split("-", 1)
        start = int(start_str) if start_str else 0
        end = int(end_str) if end_str else file_size - 1
        end = min(end, file_size - 1)
        length = end - start + 1

        with open(target, "rb") as f:
            f.seek(start)
            data = f.read(length)

        return Response(
            content=data,
            status_code=206,
            headers={
                "Content-Range": f"bytes {start}-{end}/{file_size}",
                "Content-Length": str(length),
                "Accept-Ranges": "bytes",
            },
            media_type="application/octet-stream",
        )

    return FileResponse(target)

# PUT /files/{subpath}: writes or replaces a file with the request body.
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

# POST /mkdir/{subpath}: creates a directory path recursively.
@app.post("/mkdir/{subpath:path}")
def create_dir(subpath: str):
    target = (BASE_DIR / subpath).resolve()
    try:
        target.mkdir(parents=True, exist_ok=True)
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Create dir error: {e}")
    return {"status": "ok"}


# DELETE /files/{subpath}: deletes a file or a directory tree.
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

if __name__ == "__main__":
    uvicorn.run(
        "main:app",
        host=HOST,
        port=PORT,
        reload=DEBUG
    )