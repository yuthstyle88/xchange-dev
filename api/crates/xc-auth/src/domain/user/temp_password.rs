//! Temporary password (design §3.4, V.9 :90-95): 16 characters, letters and digits only, never `0 O o 1 l I`
//! (ruling #564 issuecomment-6246076019 Q3). Always holds at least one digit, one upper-case and one lower-case
//! letter (#5952). No Display, Serialize or Clone.

use ring::rand::{SecureRandom, SystemRandom};

const ALPHABET: &[u8; 56] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz";
const LEN: usize = 16;

pub struct TempPassword(String);

impl TempPassword {
    /// Unbiased (rejection sampling) from `ring::rand::SystemRandom`.
    /// Repeats draw until the password holds at least one digit, one upper and one lower letter.
    pub fn generate() -> TempPassword {
        loop {
            let p = Self::draw();
            let b = p.0.as_bytes();
            if b.iter().any(u8::is_ascii_digit)
                && b.iter().any(u8::is_ascii_uppercase)
                && b.iter().any(u8::is_ascii_lowercase)
            {
                return p;
            }
        }
    }

    fn draw() -> TempPassword {
        let rng = SystemRandom::new();
        let mut out = String::with_capacity(LEN);
        let mut buf = [0u8; 32];
        while out.len() < LEN {
            rng.fill(&mut buf).expect("SystemRandom::fill");
            for &b in &buf {
                let idx = usize::from(b & 0x3f);
                if idx < ALPHABET.len() {
                    out.push(char::from(ALPHABET[idx]));
                    if out.len() == LEN {
                        break;
                    }
                }
            }
        }
        TempPassword(out)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for TempPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TempPassword(..)")
    }
}
