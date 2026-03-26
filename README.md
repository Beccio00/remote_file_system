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
For detailed CLI options, run `cargo run --help`.

### Connect To A Remote Server (Same LAN)

From the repository root, run the client package and point `--server-url` to the server host/IP and port.

**Unix (Linux/macOS mountpoint under `/tmp/mnt`)**

```bash
cargo run -p client -- /tmp/mnt/remote-fs --server-url http://192.168.1.50:8000
```

**Windows (drive letter mountpoint, e.g. `R:`)**

```powershell
cargo run -p client -- R: --server-url http://192.168.1.50:8000
```

Replace `192.168.1.50:8000` with the actual IP and port of the machine running the server.

### Linux

```bash
cargo build
mkdir -p /tmp/mnt
cargo run -- /tmp/mnt
```

### macOS

```bash
cargo build
mkdir -p /tmp/mnt/remote-fs
cargo run -- /tmp/mnt/remote-fs
```

### Windows

**1. Install prerequisites (PowerShell as Administrator)**

```powershell
winget install -e --id WinFsp.WinFsp
winget install -e --id Microsoft.VisualStudio.2022.Community --override "--add Microsoft.VisualStudio.Workload.NativeDesktop --add Microsoft.VisualStudio.Component.VC.Llvm.Clang --add Microsoft.VisualStudio.Component.Windows11SDK.22000"
```

If Visual Studio is already installed, open **Visual Studio Installer** and ensure these components are present:

- **Desktop development with C++**
- **C++ Clang tools for Windows**
- **Windows 10/11 SDK**

**2. Set `LIBCLANG_PATH` once (PowerShell)**

```powershell
setx LIBCLANG_PATH "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\bin"
```

Close and reopen the terminal after `setx`.

**3. Build (inside `client/`)**

```powershell
cd client
cargo clean
cargo build
```

`cargo clean` is recommended on first run (or after toolchain/dependency changes) to regenerate WinFSP bindings correctly.

**4. Run and mount**

```powershell
cargo run -- R:
```

`R:` can be any unused drive letter.

If `cargo build` still reports `Unable to find libclang`, run in the same terminal:

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\bin"
cargo clean
cargo build
```

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
  --daemon                 Run in background
  --unmount                Request clean unmount of a Windows daemon mountpoint
```

## Unmount

- **Linux/macOS**: `fusermount -u /tmp/mnt` or `Ctrl+C`
- **Windows**: `cargo run -- R: --unmount` or `Ctrl+C` 
- **macOS**: `diskutil unmount /tmp/mnt/remote-fs`