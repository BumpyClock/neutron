fn main() {
    gpui_component_manifest::build::emit_identity().expect("invalid [package.metadata.gpui-app]");
}
