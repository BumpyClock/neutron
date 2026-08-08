// Generated from protocol/weston-test.xml in Weston 16.0.0 commit
// d1882b0a544ae2197b597a6e39478e719bc54302. This module is conformance-only.
pub(crate) mod protocol {
    #![allow(dead_code, missing_docs)]
    #![allow(non_camel_case_types, non_upper_case_globals, non_snake_case)]
    #![allow(unused_imports, unused_unsafe, unused_variables)]

    use wayland_client;
    use wayland_client::protocol::*;

    pub(crate) mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/weston-test.xml");
    }

    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocol/weston-test.xml");
}
