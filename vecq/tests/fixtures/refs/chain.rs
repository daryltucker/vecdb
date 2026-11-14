fn alpha(x: u8) -> Option<u8> { Some(x) }

fn beta() -> u8 {
    alpha(1).map(|v| v + 1).unwrap_or(0)
}
