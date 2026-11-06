//! Core addresses (herdr-panorama `host::context::valid_address`): opaque,
//! qualified identifiers, never names or shell fragments.

/// `tN`, or `machine/tN` for another machine's terminal.
pub fn valid(address: &str) -> bool {
    let mut parts = address.split('/');
    let first = parts.next().unwrap_or("");
    let (machine, local) = match parts.next() {
        Some(local) => (Some(first), local),
        None => (None, first),
    };
    address.len() <= 90
        && parts.next().is_none()
        && machine.is_none_or(|m| {
            !m.is_empty()
                && m.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
        && local.strip_prefix('t').is_some_and(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
        })
}

/// Is `address` another machine's? A pid there proves nothing here.
pub fn remote(address: &str) -> bool {
    address.contains('/')
}
