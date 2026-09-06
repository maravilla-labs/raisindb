The `main.wasm` artifact that belongs beside this `.node.yaml` is a **build
output** and is not committed. Produce it with:

```sh
raisindb function build examples/wasm-functions/wasm/demo/greet-go
```

which runs the TinyGo build declared in that project's `raisin.build.yaml` and
copies the component here.
