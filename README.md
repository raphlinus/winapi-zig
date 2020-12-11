# winapi-zig

A port of [winapi-rs] to Zig. In its current state, it's a sketch, and doesn't actually work.

The theory is that winapi-rs is something of a definitive representation of the winapi surface area in "modern" types, in other words not C/C++ headers. Thus, translating it from Rust sources to Zig makes sense.

What's in this repo now is the beginnings of a translation script, using the `syn` crate to parse the original Rust, and just printing out translated Zig code. It translates basic structs and function calls, but is missing more sophisticated aspects, and it hasn't yet been run end-to-end.

## License

As in winapi-rs itself, the license is MIT or Apache 2.0, at your choice.

[winapi-rs]: https://github.com/retep998/winapi-rs
