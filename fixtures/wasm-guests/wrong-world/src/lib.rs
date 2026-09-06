//! `wrong_world.wasm` — a valid component that exports `run` instead of
//! `handler`. Instantiation must fail with a message naming the missing export.

wit_bindgen::generate!({
    inline: r#"
        package raisin:wrong-world;

        world function {
            export run: func() -> string;
        }
    "#,
    world: "function",
});

struct Component;

impl Guest for Component {
    fn run() -> String {
        "this is not the export the host wants".to_string()
    }
}

export!(Component);
