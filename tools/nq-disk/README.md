# nq-disk

Cross-platform host tool for native-qemu USB sticks.

**Self-contained:** ext4 (vendored **lwext4** C library) and GPT partitioning
(pure Rust **`gpt`** crate) are **compiled into the binary**. End users do
**not** need Homebrew, e2fsprogs, or gptfdisk — only **sudo** (everything needs
root: flash, config editor, file manager, load image, unmount).

## Features

1. **Flash ISO to USB** — guided steps; system disk excluded  
   Write ISO → verify → GPT data partition → bundled mkfs.ext4 → seed `config.toml` + `image.qcow2`
2. **File manager** — list / put / copy-out / delete on the data volume  
3. **Edit config** — nano-like editor, TOML highlight, schema completion, validate on save  
4. **Load image** — host file → always `image.qcow2`  
5. **Unmount USB**

## Run

Without root the process **exits immediately** with instructions — no menu.

```sh
# Always use sudo (macOS / Linux)
sudo ./nq-disk

# From this repo:
sudo cargo run -p nq-disk --release

sudo ./nq-disk -- flash --iso ./native-qemu-x86_64.iso --disk /dev/rdisk4 --yes
sudo ./nq-disk -- status
```

## Flash pipeline

1. Preflight: root + disk size (no external FS tools)  
2. Unmount target  
3. Raw-write ISO + SHA-256 verify  
4. **Pure-Rust GPT**: add Linux partition `native-qemu` in free space  
5. **Bundled lwext4**: `mkfs` + write `config.toml` / `image.qcow2` at partition byte offset  
   (does not wait for OS partition nodes)  

## Editor shortcuts

| Key | Action |
|-----|--------|
| Ctrl+O | Save (validate first) |
| Ctrl+X | Exit |
| Ctrl+W | Search |
| Ctrl+N | Find next |
| Ctrl+Space / Tab | Completions |
| Ctrl+G | Help |

## Data volume layout

```
config.toml
image.qcow2
```

Label: **`native-qemu`** (ext4). The appliance agent mounts this at boot.
