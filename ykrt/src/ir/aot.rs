use deku::prelude::*;

#[derive(Debug, PartialEq, DekuRead, DekuWrite)]
#[deku(magic = b"\xed\xd5\xf0\x0d")]
struct Header
    version: usize
}

struct AOTReader {
}

impl AOTReader {
    fn new(data: &[u8]) {
    }
}

#[cfg(test)]
mod tests {
    fn read() {}
}
