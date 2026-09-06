# greet-ts

The built artifact lands here as `main.wasm`; the source is
`../../../../../wasm/demo/greet-ts` and is not part of the package
(see `.rapignore`).

```sh
raisindb function build wasm/demo/greet-ts
```

The same artifact also carries a `shout` handler — a second Function node can
point at it with `entry_file: ../greet-ts/main.wasm:shout` without building
anything twice.
