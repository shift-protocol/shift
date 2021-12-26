# SHIFT Reference Implementation

&rarr; [All downloads (static binaries)](https://nightly.link/shift-protocol/shift/workflows/build/main)

---

## Usage

Connect to your remote target in a SHIFT-enabled terminal, or wrap your SSH session using `shift-host`:

```bash
shift-host --directory /local/path/for/downloads -- ssh user@host
```

On the remote host, run `shift-client send <paths>` or `shift-client receive`. The reference host will always send the first item from the `directory` argument when a download is requested.
