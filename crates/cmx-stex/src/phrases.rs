//! Phrase bank (`data/phrases.txt`) and the variance policies (STEX-DESIGN §5). Every choice comes
//! from a seeded SplitMix64, so `--natural`, `--varied` and even `--random` are reproducible:
//! the same file and flags always give byte-identical LaTeX.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Variance {
    Fixed,
    #[default]
    Natural,
    Varied,
    Random,
}

impl Variance {
    pub fn parse(s: &str) -> Option<Variance> {
        Some(match s {
            "fixed" => Variance::Fixed,
            "natural" => Variance::Natural,
            "varied" => Variance::Varied,
            "random" => Variance::Random,
            _ => return None,
        })
    }
}

pub struct Template {
    pub weight: f64,
    pub text: &'static str,
    /// Slot letters used by the template.
    pub slots: Vec<char>,
}

static BANK: OnceLock<HashMap<&'static str, Vec<Template>>> = OnceLock::new();

pub fn bank() -> &'static HashMap<&'static str, Vec<Template>> {
    BANK.get_or_init(|| {
        let mut map: HashMap<&'static str, Vec<Template>> = HashMap::new();
        let mut role: Option<&'static str> = None;
        for line in include_str!("../data/phrases.txt").lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if let Some(r) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                role = Some(r);
                map.entry(r).or_default();
                continue;
            }
            let (Some(r), Some((w, text))) = (role, line.split_once('\t')) else { continue };
            let slots = text
                .match_indices('{')
                .filter_map(|(i, _)| text[i + 1..].chars().next().filter(|c| c.is_ascii_uppercase()))
                .collect();
            map.entry(r).or_default().push(Template { weight: w.trim().parse().unwrap_or(1.0), text, slots });
        }
        map
    })
}

/// SplitMix64: tiny, fast, good enough statistical quality for phrase choice.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A seed that differs between runs (for --varied / --random without `--seed`).
pub fn entropy_seed() -> u64 {
    let t =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let local = 0u8;
    t ^ ((std::process::id() as u64) << 32) ^ (&local as *const u8 as u64).rotate_left(17)
}

/// FNV-1a 64-bit (statement hashes for per-sentence seeding).
pub fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

pub struct Picker {
    pub variance: Variance,
    rng: Rng,
    base: u64,
    /// Recently used template indices per role (most recent last).
    history: HashMap<&'static str, Vec<usize>>,
}

impl Picker {
    pub fn new(variance: Variance, seed: u64) -> Picker {
        Picker { variance, rng: Rng::new(seed), base: seed, history: HashMap::new() }
    }

    /// Start a statement. For --fixed and --natural the choice depends only on the statement's own
    /// text (plus the global seed), so output is reproducible and local edits stay local. --varied
    /// and --random keep one stream (seeded from entropy unless `--seed` is given).
    pub fn reseed_for(&mut self, statement_hash: u64) {
        if matches!(self.variance, Variance::Fixed | Variance::Natural) {
            self.rng = Rng::new(self.base ^ statement_hash);
        }
    }

    /// Choose a template for `role` among those whose slots are all in `avail`.
    pub fn pick(&mut self, role: &'static str, avail: &[char]) -> &'static str {
        let Some(list) = bank().get(role) else { return "{P}" };
        let ok: Vec<usize> = (0..list.len()).filter(|&i| list[i].slots.iter().all(|s| avail.contains(s))).collect();
        if ok.is_empty() {
            return "{P}";
        }
        let hist = self.history.entry(role).or_default();
        let idx = match self.variance {
            Variance::Fixed => ok[0],
            Variance::Natural => {
                // corpus-like weights; damp an immediate repeat so "Then … Then … Then" is rare
                let last = hist.last().copied();
                let w: Vec<f64> = ok
                    .iter()
                    .map(|&i| list[i].weight * if Some(i) == last && ok.len() > 1 { 0.35 } else { 1.0 })
                    .collect();
                ok[weighted(&mut self.rng, &w)]
            }
            Variance::Varied => {
                let recent: Vec<usize> = hist.iter().rev().take(3).copied().collect();
                let fresh: Vec<usize> = ok.iter().copied().filter(|i| !recent.contains(i)).collect();
                let pool = if fresh.is_empty() { ok.clone() } else { fresh };
                let w: Vec<f64> = pool.iter().map(|&i| list[i].weight.powf(0.35)).collect();
                pool[weighted(&mut self.rng, &w)]
            }
            Variance::Random => ok[(self.rng.next_u64() % ok.len() as u64) as usize],
        };
        hist.push(idx);
        if hist.len() > 8 {
            hist.remove(0);
        }
        list[idx].text
    }
}

fn weighted(rng: &mut Rng, w: &[f64]) -> usize {
    let total: f64 = w.iter().sum();
    let mut x = rng.unit() * total;
    for (i, wi) in w.iter().enumerate() {
        if x < *wi {
            return i;
        }
        x -= wi;
    }
    w.len() - 1
}

/// Substitute `{P}`-style slots.
pub fn fill(template: &str, slots: &[(char, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        if let Some(c) = tail.chars().next()
            && tail[c.len_utf8()..].starts_with('}')
            && let Some((_, v)) = slots.iter().find(|(k, _)| *k == c)
        {
            out.push_str(v);
            rest = &tail[c.len_utf8() + 1..];
        } else {
            out.push('{');
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_loads_and_fixed_is_first() {
        let b = bank();
        assert!(b.len() > 40, "roles: {}", b.len());
        let mut p = Picker::new(Variance::Fixed, 1);
        assert_eq!(p.pick("goal", &['P']), "We show that {P}");
        assert_eq!(p.pick("contradiction", &[]), "This is a contradiction");
        assert_eq!(fill("Let {X} and {P}.", &[('X', "$x$"), ('P', "$y$")]), "Let $x$ and $y$.");
    }

    #[test]
    fn deterministic_and_slot_filtered() {
        let run = |v| {
            let mut p = Picker::new(v, 42);
            (0..20).map(|_| p.pick("then", &['P'])).collect::<Vec<_>>()
        };
        assert_eq!(run(Variance::Natural), run(Variance::Natural));
        assert_eq!(run(Variance::Random), run(Variance::Random));
        let n = run(Variance::Natural);
        assert!(n.iter().collect::<std::collections::HashSet<_>>().len() > 3, "natural should vary: {n:?}");
        let mut p = Picker::new(Variance::Random, 7);
        for _ in 0..50 {
            assert!(!p.pick("contradiction", &[]).contains("{A}"), "slot {{A}} not available");
        }
    }
}
