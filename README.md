# Remote File System

A client/server application that mounts a remote filesystem as a local drive. The server exposes files over HTTP (FastAPI), the client mounts them using FUSE (Linux/macOS) or WinFSP (Windows).

## Structure

```
server/          → Python server (FastAPI + uvicorn)
client/          → Rust client (FUSE / WinFSP)
```

## Server

Requires Python 3.10+.

```bash
cd server
pip install -r requirements.txt
python main.py
```

Starts on `http://127.0.0.1:8000`, serving files from `server/storage/`.

---

## Client

Requires [Rust](https://rustup.rs/) and OS-specific dependencies.

### Linux

```bash
cargo build
mkdir -p /tmp/mnt
cargo run -- /tmp/mnt
```

### macOS

_TODO_

### Windows

**1. Install WinFSP**

Download and install the `.msi` from https://winfsp.dev/rel/. Default installation is fine.

**2. Install Visual Studio Build Tools**

Install [Visual Studio 2022](https://visualstudio.microsoft.com/) (Community) with these components:

- **Desktop development with C++** (workload)
- **Windows SDK** (10 or 11)
- **C++ Clang tools for Windows** (individual component — needed by `bindgen` for `libclang.dll`)

**3. Set up the build environment**

Before compiling, load the MSVC environment and set `LIBCLANG_PATH`. In PowerShell:

```powershell
# Load MSVC environment
cmd /c "call `"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat`" >nul 2>&1 && set" |
  ForEach-Object { if ($_ -match '^([^=]+)=(.*)$') { [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2]) } }

# Set libclang path
$env:LIBCLANG_PATH = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\bin"
```

> Paths may vary depending on your VS installation. This step may not be needed if you're using a **Developer PowerShell for VS 2022**.

**4. Build and mount**

```powershell
cargo build
cargo run -- R:
```

`R:` can be any unused drive letter.

---

## CLI Options

```
cargo run -- <MOUNTPOINT> [OPTIONS]

Options:
  --server-url <URL>       Server URL (default: http://127.0.0.1:8000)
  --dir-cache-ttl <SEC>    Directory cache TTL in seconds (default: 5)
  --file-cache-ttl <SEC>   File cache TTL in seconds (default: 10)
  --max-cache-mb <MB>      Max file cache size in MB (default: 64)
  --no-cache               Disable caching
  --daemon                 Run in background (Unix only)
```

## Unmount

- **Linux/macOS**: `fusermount -u /tmp/mnt` or `Ctrl+C`
- **Windows**: `Ctrl+C` in the terminal running the client
