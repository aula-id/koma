# koma-sec Toolkit — System Binary Dependencies

This document lists the external binaries the security daemon shells out to,
grouped by domain. The daemon starts and the IPC socket binds regardless of
whether these are installed, but every tool that calls a missing binary will
return an error. Install what you need; leave the rest.

---

## WEB

| Binary / service | Tool module      | Install hint                                                         |
|------------------|------------------|----------------------------------------------------------------------|
| `sqlmap`         | sec_sqlmap       | `apt install sqlmap`  or  `pip install sqlmap`                       |
| `nuclei`         | sec_nuclei       | `go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest` |
| `ffuf`           | sec_ffuf         | `go install github.com/ffuf/ffuf/v2@latest`                          |
| `dalfox`         | sec_dalfox       | `go install github.com/hahwul/dalfox/v2@latest`                      |
| OWASP ZAP daemon | sec_zap          | Download ZAP from https://www.zaproxy.org/ and start in daemon mode: `zap.sh -daemon -port 8080 -config api.key=<key>` |
| Chromium (playwright) | sec_xss_confirm | `pip install playwright && playwright install chromium`          |

### Notes

- `nuclei` templates are fetched/updated automatically on first run via
  `nuclei -update-templates`. Ensure outbound HTTPS is allowed.
- `ffuf` wordlists are **not** bundled; point the tool at a local wordlist path
  (e.g. `/usr/share/seclists/Discovery/Web-Content/common.txt`).
- ZAP must already be running and listening before `sec_zap` is called.
  The module connects to `http://localhost:8080` by default.
- `dalfox` is Go-based; make sure `$GOPATH/bin` (or `$HOME/go/bin`) is on
  `$PATH`.

---

## CRYPTO

| Binary / service   | Tool module  | Install hint                                                                 |
|--------------------|--------------|------------------------------------------------------------------------------|
| `sage`             | sec_sage     | Install SageMath: `apt install sagemath`  or  https://www.sagemath.org/     |
| `RsaCtfTool`       | sec_rsa      | `pip install RsaCtfTool`  or  `git clone https://github.com/RsaCtfTool/RsaCtfTool` |
| `hashcat`          | sec_crack    | `apt install hashcat`  or  https://hashcat.net/hashcat/                      |
| `hashid` / `name-that-hash` | sec_hashid | `pip install hashid`  or  `pip install name-that-hash`           |
| _(python)_ `z3-solver` | sec_z3  | `pip install z3-solver`                                                      |
| _(python)_ `fpylll`    | sec_lattice | `pip install fpylll`  (may require `libfplll-dev` on Debian/Ubuntu)        |

### Notes

- `sage` must be on `$PATH` as the `sage` binary; `sec_sage` shells out to it
  directly. On some distros the package is `sagemath`.
- `RsaCtfTool` can be used as a Python import or as a CLI binary; `sec_rsa`
  uses the CLI form — ensure the script is executable and on `$PATH`.
- `hashcat` requires a compatible GPU driver for GPU acceleration; CPU-only
  mode (`-D 1`) is the fallback and is slower.
- `fpylll` wheels are available on PyPI for Linux x86-64; on other platforms
  you may need to build from source (`apt install libfplll-dev` first).
- `z3-solver` and `fpylll` are listed in `requirements.txt` and are installed
  with the daemon's normal `pip install -r requirements.txt`.

---

## WEB-RE

| Binary / service        | Tool module    | Install hint                                                                         |
|-------------------------|----------------|--------------------------------------------------------------------------------------|
| `node` / `npx`          | sec_jsdeobf, sec_unmin | `apt install nodejs npm`  or  https://nodejs.org/ (LTS recommended)      |
| `webcrack` (npx)        | sec_jsdeobf, sec_unmin | `npm install -g webcrack`  (run via `npx webcrack`)                      |
| `wasm-decompile` / `wasm2wat` | sec_wasm | Install [wabt](https://github.com/WebAssembly/wabt): `apt install wabt`  or  build from source |
| _(python)_ `jsbeautifier` | sec_jsdeobf, sec_unmin, sec_sourcemap | `pip install jsbeautifier`                              |

### Notes

- `node` and `npx` are required for JavaScript deobfuscation (`sec_jsdeobf`)
  and unminification (`sec_unmin`). Ensure they are on `$PATH`.
- `webcrack` is invoked via `npx webcrack`; install globally with
  `npm install -g webcrack` or rely on `npx` auto-fetch (requires internet on
  first run).
- `wabt` provides `wasm-decompile` and `wasm2wat`; `sec_wasm` shells out to
  whichever is available. On Debian/Ubuntu: `apt install wabt`.
- `jsbeautifier` is a Python package listed in `requirements.txt` and installed
  with the normal `pip install -r requirements.txt`.
- `sec_sourcemap` uses `jsbeautifier` (Python) only — no additional system
  binary is required beyond Node/npm for source map fetching.

---

---

## PWN

| Binary / service        | Tool module  | Install hint                                                                              |
|-------------------------|--------------|-------------------------------------------------------------------------------------------|
| `checksec`              | sec_triage   | `pip install checksec.sh`  or  `apt install checksec`                                     |
| `ROPgadget`             | sec_rop      | `pip install ROPgadget`                                                                   |
| `one_gadget`            | sec_rop      | `gem install one_gadget`  (requires Ruby)                                                 |
| _(python)_ `pwntools`   | sec_triage, sec_rop, sec_pwntmpl | `pip install pwntools>=4.15`  (already in `requirements.txt`)   |

### Notes

- `checksec` is used by `sec_triage` to enumerate binary protections (NX, PIE,
  RELRO, canary, FORTIFY). Both the `checksec.sh`-based PyPI wrapper and the
  native `apt` package expose a compatible CLI; either works.
- `ROPgadget` must be on `$PATH` as `ROPgadget`; installed via
  `pip install ROPgadget`. Used by `sec_rop` to enumerate ROP gadgets from an
  ELF or raw binary.
- `one_gadget` is a Ruby gem that finds one-shot `execve("/bin/sh", ...)` gadgets
  in libc. Install with `gem install one_gadget`; requires Ruby >= 2.6.
- `pwntools` is a Python library (not a system binary) and is already listed in
  `requirements.txt`. It is used across all three PWN modules for ELF parsing,
  process/socket I/O, and exploit template generation.

---

> The toolkit is inert until the binaries above are installed and (where
> required) running. No binary is invoked at daemon startup — only when a
> matching tool is called through the IPC socket.
