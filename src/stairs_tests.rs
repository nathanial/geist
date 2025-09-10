// Placeholder to satisfy cfg(test) module import in main.rs during workspace tests.
// Real tests live under crate modules.

#[cfg(test)]
mod dummy {
    #[test]
    fn placeholder_compiles() {
        assert!(true);
    }
}
