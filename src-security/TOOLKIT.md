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

## CRYPTO / MATH  _(binaries — not yet documented)_

Entries for `sec_rsa`, `sec_factor`, `sec_lattice`, `sec_sage`, `sec_z3`, and
`sec_hashid` will be added in a later commit once the crypto domain is wired.

---

## WEB-RE / PWN  _(binaries — not yet documented)_

Entries for `sec_sourcemap`, `sec_jsdeobf`, `sec_unmin`, `sec_wasm`,
`sec_rop`, `sec_pwntmpl`, `sec_crack`, `sec_decode`, and `sec_triage` will be
added once the web-re/pwn domain is wired.

---

> The toolkit is inert until the binaries above are installed and (where
> required) running. No binary is invoked at daemon startup — only when a
> matching tool is called through the IPC socket.
