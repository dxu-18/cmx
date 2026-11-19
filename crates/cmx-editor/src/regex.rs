//! A small regular-expression engine for search and replace (no external crates).
//!
//! Syntax (Rust/PCRE-like, matched against one line at a time):
//! literals, `.`, classes `[a-z_]` / `[^…]` with `\d \w \s` inside, escapes `\d \D \w \W \s \S
//! \b \B \t \n \xHH` and escaped punctuation, anchors `^ $`, groups `( )`, non-capturing `(?: )`,
//! named groups `(?<n> )` / `(?P<n> )` (captured by number), alternation `|`, quantifiers
//! `* + ? {n} {n,} {n,m}` with lazy `?` suffixes, and a leading `(?i)` for case-insensitivity.
//!
//! Matching is a Pike VM (Thompson NFA simulation with priorities): leftmost-first semantics like
//! Perl, captures included, and time linear in the line length (no catastrophic backtracking).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Start,
    End,
    WordB,
    NotWordB,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Perl {
    Digit,
    Word,
    Space,
}

#[derive(Clone, Debug)]
enum Item {
    Range(char, char),
    Perl(Perl, bool),
}

#[derive(Clone, Debug)]
struct Class {
    items: Vec<Item>,
    negated: bool,
}

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class(Class),
    Cat(Vec<Node>),
    Alt(Vec<Node>),
    Group(Box<Node>, Option<usize>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool },
    Look(Look),
}

#[derive(Clone, Debug)]
enum Inst {
    Char(char),
    Any,
    Class(usize),
    /// Try the first target before the second.
    Split(usize, usize),
    Jmp(usize),
    Save(usize),
    Assert(Look),
    Match,
}

#[derive(Clone, Debug)]
pub struct Regex {
    prog: Vec<Inst>,
    classes: Vec<Class>,
    nsave: usize,
    icase: bool,
}

/// Capture offsets: `[start0, end0, start1, end1, …]` (group 0 is the whole match).
pub type Caps = Vec<Option<usize>>;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

impl Perl {
    fn matches(self, c: char) -> bool {
        match self {
            Perl::Digit => c.is_ascii_digit(),
            Perl::Word => is_word(c),
            Perl::Space => c.is_whitespace(),
        }
    }
}

impl Class {
    fn matches(&self, c: char, icase: bool) -> bool {
        let hit = |c: char| {
            self.items.iter().any(|it| match *it {
                Item::Range(a, b) => a <= c && c <= b,
                Item::Perl(p, neg) => p.matches(c) != neg,
            })
        };
        let m = hit(c) || (icase && (hit(fold(c)) || c.to_uppercase().next().is_some_and(hit)));
        m != self.negated
    }
}

// ------------------------------------------------------------------------------------- parser

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    groups: usize,
}

const MAX_REPEAT: u32 = 1000;

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn alt(&mut self) -> Result<Node, String> {
        let mut alts = vec![self.cat()?];
        while self.eat('|') {
            alts.push(self.cat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { Node::Alt(alts) })
    }

    fn cat(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Cat(items),
        })
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return None;
        }
        self.s[start..self.i].iter().collect::<String>().parse().ok()
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let mut node = atom;
        loop {
            let (min, max) = match self.peek() {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                Some('{') => {
                    let save = self.i;
                    self.i += 1;
                    let Some(min) = self.number() else {
                        self.i = save; // a literal '{'
                        return Ok(node);
                    };
                    let max = if self.eat(',') { self.number() } else { Some(min) };
                    if !self.eat('}') {
                        return Err("unclosed {".into());
                    }
                    if min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT || m < min) {
                        return Err("bad repetition count".into());
                    }
                    self.i -= 1; // re-consumed below
                    (min, max)
                }
                _ => return Ok(node),
            };
            self.i += 1;
            if matches!(node, Node::Look(_) | Node::Empty) {
                return Err("nothing to repeat".into());
            }
            let greedy = !self.eat('?');
            node = Node::Repeat { node: Box::new(node), min, max, greedy };
        }
    }

    fn escape(&mut self) -> Result<Node, String> {
        let c = self.peek().ok_or("trailing backslash")?;
        self.i += 1;
        let perl = |p: Perl, neg: bool| Node::Class(Class { items: vec![Item::Perl(p, neg)], negated: false });
        Ok(match c {
            'd' => perl(Perl::Digit, false),
            'D' => perl(Perl::Digit, true),
            'w' => perl(Perl::Word, false),
            'W' => perl(Perl::Word, true),
            's' => perl(Perl::Space, false),
            'S' => perl(Perl::Space, true),
            'b' => Node::Look(Look::WordB),
            'B' => Node::Look(Look::NotWordB),
            other => Node::Char(self.escaped_char(other)?),
        })
    }

    fn escaped_char(&mut self, c: char) -> Result<char, String> {
        Ok(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'f' => '\x0c',
            'v' => '\x0b',
            '0' => '\0',
            'x' => {
                let hex: String = self.s[self.i..].iter().take(2).collect();
                let v = u32::from_str_radix(&hex, 16).map_err(|_| "bad \\x escape")?;
                self.i += 2;
                char::from_u32(v).ok_or("bad \\x escape")?
            }
            c if c.is_ascii_alphanumeric() => return Err(format!("unknown escape \\{c}")),
            c => c,
        })
    }

    fn class(&mut self) -> Result<Node, String> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().ok_or("unclosed [")?;
            self.i += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let e = self.peek().ok_or("unclosed [")?;
                self.i += 1;
                match e {
                    'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                        let p = match e.to_ascii_lowercase() {
                            'd' => Perl::Digit,
                            'w' => Perl::Word,
                            _ => Perl::Space,
                        };
                        items.push(Item::Perl(p, e.is_ascii_uppercase()));
                        continue;
                    }
                    _ => self.escaped_char(e)?,
                }
            } else {
                c
            };
            // a range a-z (a '-' before ']' is literal)
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                self.i += 1;
                let mut hi = self.peek().ok_or("unclosed [")?;
                self.i += 1;
                if hi == '\\' {
                    let e = self.peek().ok_or("unclosed [")?;
                    self.i += 1;
                    hi = self.escaped_char(e)?;
                }
                if hi < lo {
                    return Err("bad class range".into());
                }
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Range(lo, lo));
            }
        }
        Ok(Node::Class(Class { items, negated }))
    }

    fn atom(&mut self) -> Result<Node, String> {
        let c = self.peek().ok_or("unexpected end")?;
        self.i += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Look(Look::Start),
            '$' => Node::Look(Look::End),
            '\\' => self.escape()?,
            '[' => self.class()?,
            '(' => {
                let index = if self.eat('?') {
                    if self.eat(':') {
                        None
                    } else if self.eat('<') || (self.eat('P') && self.eat('<')) {
                        while self.peek().is_some_and(|c| c != '>') {
                            self.i += 1;
                        }
                        if !self.eat('>') {
                            return Err("unclosed group name".into());
                        }
                        self.groups += 1;
                        Some(self.groups)
                    } else {
                        return Err("unsupported group syntax (?".into());
                    }
                } else {
                    self.groups += 1;
                    Some(self.groups)
                };
                let inner = self.alt()?;
                if !self.eat(')') {
                    return Err("unclosed (".into());
                }
                Node::Group(Box::new(inner), index)
            }
            ')' => return Err("unmatched )".into()),
            '*' | '+' | '?' => return Err("nothing to repeat".into()),
            c => Node::Char(c),
        })
    }
}

// ----------------------------------------------------------------------------------- compiler

struct Compiler {
    prog: Vec<Inst>,
    classes: Vec<Class>,
    icase: bool,
}

impl Compiler {
    fn emit(&mut self, i: Inst) -> usize {
        self.prog.push(i);
        self.prog.len() - 1
    }

    fn node(&mut self, n: &Node) -> Result<(), String> {
        if self.prog.len() > 100_000 {
            return Err("pattern too large".into());
        }
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                let c = if self.icase { fold(*c) } else { *c };
                self.emit(Inst::Char(c));
            }
            Node::Any => {
                self.emit(Inst::Any);
            }
            Node::Class(c) => {
                self.classes.push(c.clone());
                let k = self.classes.len() - 1;
                self.emit(Inst::Class(k));
            }
            Node::Look(l) => {
                self.emit(Inst::Assert(*l));
            }
            Node::Cat(v) => {
                for x in v {
                    self.node(x)?;
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (k, x) in v.iter().enumerate() {
                    if k + 1 < v.len() {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(x)?;
                        jumps.push(self.emit(Inst::Jmp(0)));
                        let next = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(x)?;
                    }
                }
                let end = self.prog.len();
                for j in jumps {
                    self.prog[j] = Inst::Jmp(end);
                }
            }
            Node::Group(inner, index) => match index {
                Some(k) => {
                    self.emit(Inst::Save(2 * k));
                    self.node(inner)?;
                    self.emit(Inst::Save(2 * k + 1));
                }
                None => self.node(inner)?,
            },
            Node::Repeat { node, min, max, greedy } => {
                for _ in 0..*min {
                    self.node(node)?;
                }
                let order = |a: usize, b: usize| if *greedy { Inst::Split(a, b) } else { Inst::Split(b, a) };
                match max {
                    None => {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(node)?;
                        self.emit(Inst::Jmp(split));
                        let end = self.prog.len();
                        self.prog[split] = order(split + 1, end);
                    }
                    Some(max) => {
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            splits.push(self.emit(Inst::Split(0, 0)));
                            self.node(node)?;
                        }
                        let end = self.prog.len();
                        for s in splits {
                            self.prog[s] = order(s + 1, end);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------------------------- VM

/// Thread list with a sparse "already added" set (one entry per instruction per step).
struct Threads {
    dense: Vec<(usize, Caps)>,
    seen: Vec<u32>,
    epoch: u32,
}

impl Threads {
    fn new(n: usize) -> Threads {
        Threads { dense: Vec::new(), seen: vec![0; n], epoch: 1 }
    }
    fn clear(&mut self) {
        self.dense.clear();
        self.epoch += 1;
        if self.epoch == u32::MAX {
            self.seen.iter_mut().for_each(|s| *s = 0);
            self.epoch = 1;
        }
    }
}

impl Regex {
    pub fn new(pattern: &str, icase: bool) -> Result<Regex, String> {
        let (pattern, icase) = match pattern.strip_prefix("(?i)") {
            Some(rest) => (rest, true),
            None => (pattern, icase),
        };
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0, groups: 0 };
        let ast = p.alt()?;
        if p.i < chars.len() {
            return Err("unmatched )".into());
        }
        let mut c = Compiler { prog: Vec::new(), classes: Vec::new(), icase };
        c.emit(Inst::Save(0));
        c.node(&ast)?;
        c.emit(Inst::Save(1));
        c.emit(Inst::Match);
        Ok(Regex { prog: c.prog, classes: c.classes, nsave: 2 * (p.groups + 1), icase })
    }

    pub fn groups(&self) -> usize {
        self.nsave / 2
    }

    fn look(l: Look, hay: &str, pos: usize) -> bool {
        let before = hay[..pos].chars().next_back();
        let after = hay[pos..].chars().next();
        match l {
            Look::Start => pos == 0,
            Look::End => pos == hay.len(),
            Look::WordB | Look::NotWordB => {
                let b = before.is_some_and(is_word) != after.is_some_and(is_word);
                b == (l == Look::WordB)
            }
        }
    }

    /// Follow ε-transitions from `pc`, adding the reachable consuming states in priority order.
    fn add(&self, list: &mut Threads, pc: usize, hay: &str, pos: usize, mut caps: Caps) {
        // explicit stack (ε-chains can be long: x{0,1000})
        // (pc, capture slot to restore once its subtree is added)
        type Frame = (usize, Option<(usize, Option<usize>)>);
        let mut stack: Vec<Frame> = vec![(pc, None)];
        while let Some((pc, restore)) = stack.pop() {
            if let Some((slot, old)) = restore {
                caps[slot] = old;
                if pc == usize::MAX {
                    continue;
                }
            }
            if list.seen[pc] == list.epoch {
                continue;
            }
            list.seen[pc] = list.epoch;
            match self.prog[pc] {
                Inst::Jmp(t) => stack.push((t, None)),
                Inst::Split(a, b) => {
                    stack.push((b, None));
                    stack.push((a, None));
                }
                Inst::Save(slot) => {
                    // restore the slot once everything reachable through it has been added
                    stack.push((usize::MAX, Some((slot, caps[slot]))));
                    caps[slot] = Some(pos);
                    stack.push((pc + 1, None));
                }
                Inst::Assert(l) => {
                    if Self::look(l, hay, pos) {
                        stack.push((pc + 1, None));
                    }
                }
                _ => list.dense.push((pc, caps.clone())),
            }
        }
    }

    /// Leftmost-first match starting at or after byte `start` of `hay`.
    pub fn captures_at(&self, hay: &str, start: usize) -> Option<Caps> {
        if start > hay.len() {
            return None;
        }
        let n = self.prog.len();
        let mut clist = Threads::new(n);
        let mut nlist = Threads::new(n);
        let mut matched: Option<Caps> = None;
        let mut pos = start;
        loop {
            if matched.is_none() {
                self.add(&mut clist, 0, hay, pos, vec![None; self.nsave]);
            }
            if clist.dense.is_empty() && matched.is_some() {
                break;
            }
            let c = hay[pos..].chars().next();
            let next = pos + c.map_or(0, char::len_utf8);
            let threads = std::mem::take(&mut clist.dense);
            for (pc, caps) in threads {
                let ok = match (&self.prog[pc], c) {
                    (Inst::Match, _) => {
                        matched = Some(caps);
                        break; // lower-priority threads are cut
                    }
                    (Inst::Char(ch), Some(c)) => *ch == if self.icase { fold(c) } else { c },
                    (Inst::Any, Some(c)) => c != '\n',
                    (Inst::Class(k), Some(c)) => self.classes[*k].matches(c, self.icase),
                    _ => false,
                };
                if ok {
                    self.add(&mut nlist, pc + 1, hay, next, caps);
                }
            }
            if c.is_none() {
                break;
            }
            pos = next;
            std::mem::swap(&mut clist, &mut nlist);
            nlist.clear();
        }
        matched
    }

    /// `(start, end)` of the first match at or after `start`.
    pub fn find_at(&self, hay: &str, start: usize) -> Option<(usize, usize)> {
        let c = self.captures_at(hay, start)?;
        Some((c[0]?, c[1]?))
    }

    /// All non-overlapping, non-empty matches.
    pub fn find_all(&self, hay: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at <= hay.len() {
            let Some((a, b)) = self.find_at(hay, at) else { break };
            if b > a {
                out.push((a, b));
                at = b;
            } else {
                // empty match: step one char
                at = a + hay[a..].chars().next().map_or(1, char::len_utf8);
            }
        }
        out
    }
}

/// Escape regex metacharacters in a literal.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Expand a replacement template: `$0`–`$9`, `${N}`, `\0`–`\9`, `$$` → `$`, `\\` → `\`,
/// `\n` / `\t`; with `amp`, a bare `&` is the whole match (vim) and `\&` a literal `&`.
pub fn expand(rep: &str, caps: &Caps, hay: &str, amp: bool) -> String {
    let group = |k: usize| -> &str {
        match (caps.get(2 * k).copied().flatten(), caps.get(2 * k + 1).copied().flatten()) {
            (Some(a), Some(b)) => &hay[a..b],
            _ => "",
        }
    };
    let mut out = String::new();
    let mut it = rep.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '$' => match it.peek().copied() {
                Some('$') => {
                    it.next();
                    out.push('$');
                }
                Some(d) if d.is_ascii_digit() => {
                    it.next();
                    out.push_str(group(d as usize - '0' as usize));
                }
                Some('{') => {
                    let rest: String = it.clone().collect();
                    match rest[1..].find('}').and_then(|e| rest[1..1 + e].parse::<usize>().ok().map(|n| (n, e))) {
                        Some((n, e)) => {
                            out.push_str(group(n));
                            for _ in 0..e + 2 {
                                it.next();
                            }
                        }
                        None => out.push('$'),
                    }
                }
                _ => out.push('$'),
            },
            '\\' => match it.next() {
                Some(d) if d.is_ascii_digit() => out.push_str(group(d as usize - '0' as usize)),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            },
            '&' if amp => out.push_str(group(0)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> Option<(usize, usize)> {
        Regex::new(p, false).unwrap().find_at(s, 0)
    }

    #[test]
    fn basics() {
        assert_eq!(m("abc", "xxabcxx"), Some((2, 5)));
        assert_eq!(m("a.c", "abc"), Some((0, 3)));
        assert_eq!(m("a|bc|d", "xbcd"), Some((1, 3)));
        assert_eq!(m("colou?r", "the color"), Some((4, 9)));
        assert_eq!(m("ab*c", "ac abbbc"), Some((0, 2)));
        assert_eq!(m("ab+c", "ac abbbc"), Some((3, 8)));
        assert_eq!(m("^foo", "foo bar foo"), Some((0, 3)));
        assert_eq!(m("foo$", "foo bar foo"), Some((8, 11)));
        assert_eq!(m("^$", ""), Some((0, 0)));
        assert_eq!(m("x{2,3}", "x xx xxxx"), Some((2, 4)));
        assert_eq!(m("x{2}", "xxxx"), Some((0, 2)));
        assert_eq!(m("x{2,}", "xxxx"), Some((0, 4)));
        assert_eq!(m("a{,", "a{,"), Some((0, 3))); // literal braces
        assert_eq!(m("nope", "abc"), None);
    }

    #[test]
    fn classes_and_escapes() {
        assert_eq!(m(r"\d+", "abc 123 x"), Some((4, 7)));
        assert_eq!(m(r"[a-c]+", "xxbcaz"), Some((2, 5)));
        assert_eq!(m(r"[^a-c ]+", "abc xyz"), Some((4, 7)));
        assert_eq!(m(r"[\d_]+", "ab_12c"), Some((2, 5)));
        assert_eq!(m(r"[-a]+", "x-a-"), Some((1, 4)));
        assert_eq!(m(r"\bcat\b", "concat cat"), Some((7, 10)));
        assert_eq!(m(r"\Bcat", "concat cat"), Some((3, 6)));
        assert_eq!(m(r"\s\S", "a  b"), Some((2, 4)));
        assert_eq!(m(r"\(x\)", "f(x)"), Some((1, 4)));
        assert_eq!(m(r"\x41", "zA"), Some((1, 2)));
        assert_eq!(m(r"é+", "café"), Some((3, 5)));
    }

    #[test]
    fn greedy_lazy_and_captures() {
        assert_eq!(m("<.*>", "<a><b>"), Some((0, 6)));
        assert_eq!(m("<.*?>", "<a><b>"), Some((0, 3)));
        assert_eq!(m("a??b", "ab"), Some((0, 2)));
        let r = Regex::new(r"(\w+)@(\w+)\.com", false).unwrap();
        let c = r.captures_at("mail bob@example.com now", 0).unwrap();
        assert_eq!(&"mail bob@example.com now"[c[2].unwrap()..c[3].unwrap()], "bob");
        assert_eq!(&"mail bob@example.com now"[c[4].unwrap()..c[5].unwrap()], "example");
        assert_eq!(r.groups(), 3);
        // the last iteration of a repeated group is captured
        let r = Regex::new(r"(a|b)+", false).unwrap();
        let c = r.captures_at("abba", 0).unwrap();
        assert_eq!((c[2], c[3]), (Some(3), Some(4)));
        // non-capturing and named groups
        let r = Regex::new(r"(?:x)(?<n>y)(?P<m>z)", false).unwrap();
        assert_eq!(r.groups(), 3);
    }

    #[test]
    fn case_insensitive_and_errors() {
        let r = Regex::new("hello [a-z]+", true).unwrap();
        assert_eq!(r.find_at("Say HELLO World", 0), Some((4, 15)));
        assert_eq!(Regex::new("(?i)abc", false).unwrap().find_at("xABC", 0), Some((1, 4)));
        for bad in ["(", "a)", "[a", "*a", "a{3,1}", r"\q", "(?=x)"] {
            assert!(Regex::new(bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_catastrophic_backtracking() {
        let r = Regex::new("(a*)*b", false).unwrap();
        let hay = "a".repeat(5000);
        let t = std::time::Instant::now();
        assert_eq!(r.find_at(&hay, 0), None);
        assert!(t.elapsed().as_secs_f64() < 2.0);
    }

    #[test]
    fn find_all_and_expand() {
        let r = Regex::new(r"(\w)(\d)", false).unwrap();
        assert_eq!(r.find_all("a1 b2 c"), vec![(0, 2), (3, 5)]);
        let c = r.captures_at("a1", 0).unwrap();
        assert_eq!(expand("$2$1-${1}\\2$$", &c, "a1", false), "1a-a1$");
        assert_eq!(expand("[&] \\&", &c, "a1", true), "[a1] &");
        assert_eq!(Regex::new("x*", false).unwrap().find_all("axxb"), vec![(1, 3)]);
        assert_eq!(escape("a.b*(c)"), r"a\.b\*\(c\)");
    }
}
