fn main() {
    neutron_components_manifest::build::emit_identity()
        .expect("invalid [package.metadata.gpui-app]");
}
