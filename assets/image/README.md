# Default guest disk (image.qcow2)

Store the multi-volume 7-Zip archive here (Git LFS recommended):

- `image.7z.001`
- `image.7z.002`

Reconstruct:

```sh
7z x assets/image/image.7z.001 -o/tmp/native-qemu-image
# produces image.qcow2
```

CI unpacks these into the x86_64 ISO as `images/image.qcow2`.
The USB data volume always uses the filename **`image.qcow2`** so you can
swap guests without editing `config.toml`.
