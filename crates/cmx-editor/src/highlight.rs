//! Table-driven syntax highlighting with a per-line carried state (block comments, multi-line
//! strings, fenced code, display math). Fast enough to re-highlight visible lines every frame.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Text,
    Keyword,
    Control,
    Type,
    Function,
    Str,
    Number,
    Comment,
    Constant,
    Macro,
    Operator,
    Heading,
    Variable,
    Tag,
    Math,
    Emph,
    Strong,
    Code,
    Link,
}

pub type Span = (u32, u32, Tok);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Plain,
    Rust,
    C,
    Cpp,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    Kotlin,
    Swift,
    CSharp,
    Shell,
    Json,
    Toml,
    Yaml,
    Markdown,
    Latex,
    Stex,
    Lean,
    Html,
    Css,
    Makefile,
    Lua,
    Ruby,
    Sql,
    Haskell,
    Zig,
}

pub struct StrDef {
    pub open: &'static str,
    pub close: &'static str,
    pub multiline: bool,
    pub escape: bool,
    /// For char literals: give up (not a string) if no close within this many chars.
    pub max_len: usize,
}

const fn sd(open: &'static str, close: &'static str, multiline: bool) -> StrDef {
    StrDef { open, close, multiline, escape: true, max_len: usize::MAX }
}
const fn ch(q: &'static str) -> StrDef {
    StrDef { open: q, close: q, multiline: false, escape: true, max_len: 3 }
}

pub struct LangDef {
    pub name: &'static str,
    pub line_comment: &'static [&'static str],
    pub block_comment: Option<(&'static str, &'static str)>,
    pub nested_comments: bool,
    pub strings: &'static [StrDef],
    pub keywords: &'static [&'static str],
    pub control: &'static [&'static str],
    pub types: &'static [&'static str],
    pub constants: &'static [&'static str],
    pub caps_types: bool,
    pub macro_bang: bool,
    pub preproc: bool,
    pub decorator: Option<u8>,
    pub var_sigil: Option<u8>,
    pub ident_dash: bool,
    pub case_insensitive: bool,
}

const BASE: LangDef = LangDef {
    name: "Plain Text",
    line_comment: &[],
    block_comment: None,
    nested_comments: false,
    strings: &[],
    keywords: &[],
    control: &[],
    types: &[],
    constants: &[],
    caps_types: false,
    macro_bang: false,
    preproc: false,
    decorator: None,
    var_sigil: None,
    ident_dash: false,
    case_insensitive: false,
};

const C_CONTROL: &[&str] = &[
    "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "goto", "try",
    "catch", "throw", "finally",
];

static RUST: LangDef = LangDef {
    name: "Rust",
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    nested_comments: true,
    strings: &[sd("b\"", "\"", true), sd("\"", "\"", true), ch("'")],
    keywords: &[
        "as",
        "async",
        "await",
        "const",
        "crate",
        "dyn",
        "enum",
        "extern",
        "fn",
        "impl",
        "in",
        "let",
        "mod",
        "move",
        "mut",
        "pub",
        "ref",
        "self",
        "Self",
        "static",
        "struct",
        "super",
        "trait",
        "type",
        "union",
        "unsafe",
        "use",
        "where",
        "macro_rules",
    ],
    control: &["if", "else", "for", "while", "loop", "match", "break", "continue", "return", "yield"],
    types: &[
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool",
        "char", "str", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    ],
    constants: &["true", "false", "None", "Some", "Ok", "Err"],
    caps_types: true,
    macro_bang: true,
    decorator: Some(b'#'),
    ..BASE
};

static C: LangDef = LangDef {
    name: "C",
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[sd("\"", "\"", false), ch("'")],
    keywords: &[
        "auto",
        "const",
        "enum",
        "extern",
        "inline",
        "register",
        "restrict",
        "sizeof",
        "static",
        "struct",
        "typedef",
        "union",
        "volatile",
        "_Bool",
        "_Atomic",
        "_Noreturn",
        "_Static_assert",
        "alignof",
    ],
    control: C_CONTROL,
    types: &[
        "void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "size_t", "ssize_t",
        "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t", "bool", "FILE",
    ],
    constants: &["NULL", "true", "false", "EOF"],
    preproc: true,
    ..BASE
};

static CPP: LangDef = LangDef {
    name: "C++",
    keywords: &[
        "auto",
        "class",
        "const",
        "constexpr",
        "consteval",
        "delete",
        "enum",
        "explicit",
        "export",
        "extern",
        "friend",
        "inline",
        "mutable",
        "namespace",
        "new",
        "noexcept",
        "operator",
        "override",
        "private",
        "protected",
        "public",
        "static",
        "static_assert",
        "struct",
        "template",
        "this",
        "typedef",
        "typename",
        "union",
        "using",
        "virtual",
        "volatile",
        "final",
        "concept",
        "requires",
        "co_await",
        "co_return",
        "co_yield",
        "sizeof",
        "decltype",
        "static_cast",
        "dynamic_cast",
        "reinterpret_cast",
        "const_cast",
    ],
    types: &[
        "void",
        "char",
        "short",
        "int",
        "long",
        "float",
        "double",
        "signed",
        "unsigned",
        "bool",
        "size_t",
        "std",
        "string",
        "vector",
        "map",
        "set",
        "unordered_map",
        "unique_ptr",
        "shared_ptr",
        "uint8_t",
        "uint32_t",
        "uint64_t",
        "int32_t",
        "int64_t",
    ],
    constants: &["nullptr", "true", "false", "NULL"],
    caps_types: true,
    ..C
};

static PYTHON: LangDef = LangDef {
    name: "Python",
    line_comment: &["#"],
    strings: &[
        sd("\"\"\"", "\"\"\"", true),
        sd("'''", "'''", true),
        sd("f\"", "\"", false),
        sd("r\"", "\"", false),
        sd("b\"", "\"", false),
        sd("\"", "\"", false),
        sd("'", "'", false),
    ],
    keywords: &[
        "and", "as", "assert", "async", "await", "class", "def", "del", "from", "global", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "self", "cls",
    ],
    control: &[
        "if", "elif", "else", "for", "while", "break", "continue", "return", "try", "except", "finally", "raise",
        "with", "yield", "match", "case",
    ],
    types: &["int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object", "type"],
    constants: &["True", "False", "None"],
    caps_types: true,
    decorator: Some(b'@'),
    ..BASE
};

static JS: LangDef = LangDef {
    name: "JavaScript",
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[sd("\"", "\"", false), sd("'", "'", false), sd("`", "`", true)],
    keywords: &[
        "async",
        "await",
        "class",
        "const",
        "constructor",
        "delete",
        "export",
        "extends",
        "from",
        "function",
        "get",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "of",
        "set",
        "static",
        "super",
        "this",
        "typeof",
        "var",
        "void",
        "as",
        "interface",
        "type",
        "enum",
        "implements",
        "private",
        "public",
        "protected",
        "readonly",
        "declare",
        "namespace",
        "abstract",
        "keyof",
    ],
    control: &[
        "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "try", "catch",
        "finally", "throw", "yield",
    ],
    types: &["number", "string", "boolean", "any", "unknown", "never", "object", "symbol", "bigint"],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
    caps_types: true,
    decorator: Some(b'@'),
    var_sigil: None,
    ..BASE
};

static GO: LangDef = LangDef {
    name: "Go",
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[sd("\"", "\"", false), sd("`", "`", true), ch("'")],
    keywords: &[
        "chan",
        "const",
        "defer",
        "func",
        "go",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "struct",
        "type",
        "var",
        "select",
    ],
    control: &["if", "else", "for", "switch", "case", "default", "break", "continue", "return", "goto", "fallthrough"],
    types: &[
        "bool",
        "byte",
        "complex64",
        "complex128",
        "error",
        "float32",
        "float64",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "rune",
        "string",
        "uint",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uintptr",
        "any",
    ],
    constants: &["true", "false", "nil", "iota"],
    ..BASE
};

static JAVA: LangDef = LangDef {
    name: "Java",
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[sd("\"\"\"", "\"\"\"", true), sd("\"", "\"", false), ch("'")],
    keywords: &[
        "abstract",
        "class",
        "enum",
        "extends",
        "final",
        "implements",
        "import",
        "instanceof",
        "interface",
        "native",
        "new",
        "package",
        "private",
        "protected",
        "public",
        "static",
        "super",
        "synchronized",
        "this",
        "throws",
        "transient",
        "volatile",
        "var",
        "record",
        "sealed",
        "permits",
        "fun",
        "val",
        "object",
        "companion",
        "data",
        "override",
        "open",
        "internal",
        "lateinit",
        "struct",
        "func",
        "let",
        "init",
        "self",
        "protocol",
        "extension",
        "guard",
        "using",
        "namespace",
        "readonly",
        "async",
        "await",
    ],
    control: &[
        "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "try", "catch",
        "finally", "throw", "when", "in", "is",
    ],
    types: &[
        "void", "boolean", "byte", "char", "short", "int", "long", "float", "double", "String", "Int", "Long",
        "Double", "Bool", "Unit", "string", "bool",
    ],
    constants: &["true", "false", "null", "nil"],
    caps_types: true,
    decorator: Some(b'@'),
    ..BASE
};

static SHELL: LangDef = LangDef {
    name: "Shell",
    line_comment: &["#"],
    strings: &[
        sd("\"", "\"", true),
        StrDef { open: "'", close: "'", multiline: true, escape: false, max_len: usize::MAX },
    ],
    keywords: &[
        "function", "local", "export", "readonly", "declare", "source", "alias", "unset", "set", "echo", "printf",
        "cd", "eval", "exec", "trap", "shift", "test",
    ],
    control: &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "return",
        "exit", "break", "continue", "select",
    ],
    constants: &["true", "false"],
    var_sigil: Some(b'$'),
    ident_dash: true,
    ..BASE
};

static JSON: LangDef = LangDef {
    name: "JSON",
    line_comment: &["//"],
    strings: &[sd("\"", "\"", false)],
    constants: &["true", "false", "null"],
    ..BASE
};

static TOML: LangDef = LangDef {
    name: "TOML",
    line_comment: &["#"],
    strings: &[
        sd("\"\"\"", "\"\"\"", true),
        sd("'''", "'''", true),
        sd("\"", "\"", false),
        StrDef { open: "'", close: "'", multiline: false, escape: false, max_len: usize::MAX },
    ],
    constants: &["true", "false"],
    ident_dash: true,
    ..BASE
};

static YAML: LangDef = LangDef {
    name: "YAML",
    line_comment: &["#"],
    strings: &[
        sd("\"", "\"", false),
        StrDef { open: "'", close: "'", multiline: false, escape: false, max_len: usize::MAX },
    ],
    constants: &["true", "false", "null", "yes", "no", "on", "off", "~"],
    ident_dash: true,
    ..BASE
};

static LEAN: LangDef = LangDef {
    name: "Lean",
    line_comment: &["--"],
    block_comment: Some(("/-", "-/")),
    nested_comments: true,
    strings: &[sd("\"", "\"", false)],
    keywords: &[
        "theorem",
        "lemma",
        "def",
        "example",
        "structure",
        "class",
        "instance",
        "inductive",
        "where",
        "namespace",
        "section",
        "end",
        "open",
        "import",
        "variable",
        "universe",
        "abbrev",
        "noncomputable",
        "private",
        "protected",
        "fun",
        "let",
        "have",
        "show",
        "from",
        "by",
        "at",
        "with",
        "calc",
        "deriving",
        "mutual",
    ],
    control: &[
        "intro",
        "intros",
        "exact",
        "apply",
        "rw",
        "simp",
        "omega",
        "ring",
        "linarith",
        "nlinarith",
        "norm_num",
        "decide",
        "constructor",
        "cases",
        "induction",
        "rcases",
        "obtain",
        "use",
        "refine",
        "exists",
        "unfold",
        "contradiction",
        "exfalso",
        "aesop",
        "positivity",
        "gcongr",
        "field_simp",
        "push_neg",
        "by_contra",
        "match",
        "if",
        "then",
        "else",
        "do",
        "return",
        "sorry",
    ],
    types: &["Nat", "Int", "Real", "Rat", "Prop", "Type", "Bool", "ZMod", "Finset", "Set", "List", "ℕ", "ℤ", "ℝ", "ℚ"],
    constants: &["True", "False", "true", "false", "rfl", "trivial"],
    caps_types: true,
    decorator: Some(b'@'),
    ..BASE
};

static CSS: LangDef = LangDef {
    name: "CSS",
    block_comment: Some(("/*", "*/")),
    strings: &[sd("\"", "\"", false), sd("'", "'", false)],
    keywords: &["important", "media", "import", "keyframes", "font-face", "supports"],
    decorator: Some(b'@'),
    ident_dash: true,
    ..BASE
};

static HTML: LangDef = LangDef {
    name: "HTML",
    block_comment: Some(("<!--", "-->")),
    strings: &[sd("\"", "\"", false), sd("'", "'", false)],
    ident_dash: true,
    ..BASE
};

static MAKE: LangDef = LangDef {
    name: "Makefile",
    line_comment: &["#"],
    strings: &[sd("\"", "\"", false), sd("'", "'", false)],
    keywords: &[
        "ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "define", "endef", "export", "override",
        "PHONY",
    ],
    var_sigil: Some(b'$'),
    ident_dash: true,
    ..BASE
};

static LUA: LangDef = LangDef {
    name: "Lua",
    line_comment: &["--"],
    block_comment: Some(("--[[", "]]")),
    strings: &[sd("\"", "\"", false), sd("'", "'", false), sd("[[", "]]", true)],
    keywords: &["and", "function", "local", "not", "or", "in"],
    control: &[
        "if", "then", "else", "elseif", "end", "for", "while", "do", "repeat", "until", "return", "break", "goto",
    ],
    constants: &["true", "false", "nil"],
    ..BASE
};

static RUBY: LangDef = LangDef {
    name: "Ruby",
    line_comment: &["#"],
    strings: &[sd("\"", "\"", true), sd("'", "'", true)],
    keywords: &[
        "alias",
        "and",
        "begin",
        "class",
        "def",
        "defined?",
        "do",
        "end",
        "module",
        "not",
        "or",
        "self",
        "super",
        "yield",
        "require",
        "attr_accessor",
        "lambda",
        "proc",
    ],
    control: &[
        "if", "elsif", "else", "unless", "case", "when", "while", "until", "for", "in", "break", "next", "redo",
        "retry", "return", "rescue", "ensure", "raise", "then",
    ],
    constants: &["true", "false", "nil"],
    caps_types: true,
    var_sigil: Some(b'@'),
    ..BASE
};

static SQL: LangDef = LangDef {
    name: "SQL",
    line_comment: &["--"],
    block_comment: Some(("/*", "*/")),
    strings: &[sd("'", "'", true), sd("\"", "\"", false)],
    keywords: &[
        "select",
        "from",
        "where",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "drop",
        "alter",
        "index",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "on",
        "group",
        "by",
        "order",
        "having",
        "limit",
        "offset",
        "as",
        "and",
        "or",
        "not",
        "in",
        "is",
        "like",
        "between",
        "distinct",
        "union",
        "all",
        "primary",
        "key",
        "foreign",
        "references",
        "default",
        "with",
        "case",
        "when",
        "then",
        "else",
        "end",
        "exists",
    ],
    types: &[
        "int",
        "integer",
        "bigint",
        "text",
        "varchar",
        "char",
        "boolean",
        "real",
        "float",
        "date",
        "timestamp",
        "serial",
        "numeric",
    ],
    constants: &["null", "true", "false"],
    case_insensitive: true,
    ..BASE
};

static HASKELL: LangDef = LangDef {
    name: "Haskell",
    line_comment: &["--"],
    block_comment: Some(("{-", "-}")),
    nested_comments: true,
    strings: &[sd("\"", "\"", false), ch("'")],
    keywords: &[
        "module",
        "import",
        "qualified",
        "as",
        "hiding",
        "where",
        "data",
        "type",
        "newtype",
        "class",
        "instance",
        "deriving",
        "let",
        "in",
        "do",
    ],
    control: &["if", "then", "else", "case", "of"],
    constants: &["True", "False", "Nothing", "Just"],
    caps_types: true,
    ..BASE
};

static ZIG: LangDef = LangDef {
    name: "Zig",
    line_comment: &["//"],
    strings: &[sd("\"", "\"", false), ch("'")],
    keywords: &[
        "const",
        "var",
        "fn",
        "pub",
        "struct",
        "enum",
        "union",
        "error",
        "comptime",
        "inline",
        "extern",
        "export",
        "test",
        "defer",
        "errdefer",
        "try",
        "usingnamespace",
        "anytype",
    ],
    control: &["if", "else", "while", "for", "switch", "break", "continue", "return", "orelse", "catch", "unreachable"],
    types: &[
        "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64", "bool", "void", "type",
        "anyerror",
    ],
    constants: &["true", "false", "null", "undefined"],
    decorator: Some(b'@'),
    ..BASE
};

/// stex structural keywords (prose drivers) and math words; see claude/STEX-DESIGN.md.
static STEX: LangDef = LangDef {
    name: "stex",
    line_comment: &["//"],
    strings: &[sd("\"", "\"", false)],
    keywords: &[
        "let",
        "fix",
        "assume",
        "suppose",
        "wts",
        "afsoc",
        "aftsoc",
        "contradiction",
        "contra",
        "then",
        "thus",
        "hence",
        "therefore",
        "note",
        "wlog",
        "implies",
        "iff",
        "and",
        "or",
        "not",
        "xor",
        "nand",
        "nor",
        "exists",
        "there",
        "for",
        "all",
        "every",
        "each",
        "any",
        "such",
        "that",
        "st",
        "by",
        "because",
        "since",
        "with",
        "define",
        "via",
        "be",
        "case",
        "qed",
        "ie",
        "be",
        "in",
        "to",
        "from",
        "of",
        "over",
        "mod",
    ],
    control: &[
        "proof",
        "claim",
        "lemma",
        "proposition",
        "theorem",
        "corollary",
        "observation",
        "remark",
        "fact",
        "sublemma",
        "aside",
        "bold",
        "italic",
        "underline",
        "bf",
        "it",
        "ul",
        "latex",
    ],
    types: &["integers", "reals", "naturals", "rationals", "complex", "field", "Z", "N", "R", "Q", "C"],
    constants: &[
        "sum",
        "prod",
        "integral",
        "integrate",
        "limit",
        "lim",
        "derivative",
        "sqrt",
        "choose",
        "union",
        "intersect",
        "intersection",
        "setminus",
        "complement",
        "empty",
        "alpha",
        "beta",
        "gamma",
        "delta",
        "epsilon",
        "theta",
        "lambda",
        "mu",
        "pi",
        "sigma",
        "phi",
        "omega",
        "infinity",
        "inf",
    ],
    ..BASE
};

pub fn def(lang: Lang) -> &'static LangDef {
    static PLAIN: LangDef = BASE;
    static TS: LangDef = LangDef { name: "TypeScript", ..JS };
    static KOTLIN: LangDef = LangDef { name: "Kotlin", ..JAVA };
    static SWIFT: LangDef = LangDef { name: "Swift", ..JAVA };
    static CSHARP: LangDef = LangDef { name: "C#", ..JAVA };
    static MD: LangDef = LangDef { name: "Markdown", ..BASE };
    static TEX: LangDef = LangDef { name: "LaTeX", ..BASE };
    match lang {
        Lang::Plain => &PLAIN,
        Lang::Rust => &RUST,
        Lang::C => &C,
        Lang::Cpp => &CPP,
        Lang::Python => &PYTHON,
        Lang::JavaScript => &JS,
        Lang::TypeScript => &TS,
        Lang::Go => &GO,
        Lang::Java => &JAVA,
        Lang::Kotlin => &KOTLIN,
        Lang::Swift => &SWIFT,
        Lang::CSharp => &CSHARP,
        Lang::Shell => &SHELL,
        Lang::Json => &JSON,
        Lang::Toml => &TOML,
        Lang::Yaml => &YAML,
        Lang::Markdown => &MD,
        Lang::Latex => &TEX,
        Lang::Stex => &STEX,
        Lang::Lean => &LEAN,
        Lang::Html => &HTML,
        Lang::Css => &CSS,
        Lang::Makefile => &MAKE,
        Lang::Lua => &LUA,
        Lang::Ruby => &RUBY,
        Lang::Sql => &SQL,
        Lang::Haskell => &HASKELL,
        Lang::Zig => &ZIG,
    }
}

impl Lang {
    pub fn name(self) -> &'static str {
        def(self).name
    }

    /// Line-comment token used by "toggle comment".
    pub fn comment_token(self) -> Option<&'static str> {
        match self {
            Lang::Latex => Some("%"),
            Lang::Html | Lang::Css | Lang::Markdown | Lang::Plain => None,
            l => def(l).line_comment.first().copied(),
        }
    }

    pub fn detect(path: &Path) -> Lang {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        match name {
            "Makefile" | "makefile" | "GNUmakefile" => return Lang::Makefile,
            "Dockerfile" | ".bashrc" | ".zshrc" | ".bash_profile" | ".profile" | ".zprofile" | "PKGBUILD" => {
                return Lang::Shell;
            }
            "Cargo.lock" => return Lang::Toml,
            "CMakeLists.txt" => return Lang::Makefile,
            _ => {}
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Lang::Rust,
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ino" | "m" | "mm" => Lang::Cpp,
            "py" | "pyi" | "pyw" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Lang::TypeScript,
            "go" => Lang::Go,
            "java" => Lang::Java,
            "kt" | "kts" => Lang::Kotlin,
            "swift" => Lang::Swift,
            "cs" => Lang::CSharp,
            "sh" | "bash" | "zsh" | "fish" | "ksh" => Lang::Shell,
            "json" | "jsonc" | "json5" => Lang::Json,
            "toml" => Lang::Toml,
            "yml" | "yaml" => Lang::Yaml,
            "md" | "markdown" | "mdx" => Lang::Markdown,
            "tex" | "sty" | "cls" | "bib" | "ltx" => Lang::Latex,
            "stex" => Lang::Stex,
            "lean" => Lang::Lean,
            "html" | "htm" | "xml" | "svg" | "vue" => Lang::Html,
            "css" | "scss" | "less" => Lang::Css,
            "mk" | "make" => Lang::Makefile,
            "lua" => Lang::Lua,
            "rb" | "rake" | "gemspec" => Lang::Ruby,
            "sql" => Lang::Sql,
            "hs" | "lhs" => Lang::Haskell,
            "zig" => Lang::Zig,
            _ => Lang::Plain,
        }
    }
}

const STR_FLAG: u32 = 0x1000;
const MD_FENCE: u32 = 0x2000;
const TEX_DMATH: u32 = 0x4000;

#[inline]
fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}
#[inline]
fn is_ident(c: u8, dash: bool) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 || (dash && c == b'-')
}

fn in_list(list: &[&str], w: &str, ci: bool) -> bool {
    if ci { list.iter().any(|k| k.eq_ignore_ascii_case(w)) } else { list.contains(&w) }
}

/// Find the end (exclusive) of a string body starting at byte `i` (just after the opener).
fn string_end(s: &str, mut i: usize, d: &StrDef) -> Option<usize> {
    let b = s.as_bytes();
    let mut count = 0;
    while i < b.len() {
        if d.escape && b[i] == b'\\' {
            i += 2;
            count += 1;
            continue;
        }
        if s[i..].starts_with(d.close) {
            return Some(i + d.close.len());
        }
        i += 1;
        while i < b.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        count += 1;
        if count > d.max_len {
            return None;
        }
    }
    None
}

/// Highlight one line. `state` carries block-comment depth / open string / fence across lines.
/// When `out` is `None` only the state is computed (faster; used to catch up long files).
pub fn highlight_line(lang: Lang, s: &str, state: u32, out: Option<&mut Vec<Span>>) -> u32 {
    match lang {
        Lang::Markdown => markdown(s, state, out),
        Lang::Latex => latex(s, state, out),
        Lang::Stex => stex(s, state, out),
        Lang::Plain => 0,
        _ => generic(def(lang), lang, s, state, out),
    }
}

/// stex: colored by the stex compiler's own lexer, so a keyword is colored exactly when the
/// compiler reads it as one (phrases only once complete; every alias of a construct in the same
/// color; see cmx_stex::highlight). State 1 = inside a `$` display block.
fn stex(s: &str, state: u32, out: Option<&mut Vec<Span>>) -> u32 {
    use cmx_stex::highlight::{Role, line_roles};
    let (roles, open) = line_roles(s, state == 1);
    if let Some(o) = out {
        for (a, b, r) in roles {
            let t = match r {
                Role::Structure => Tok::Keyword,
                Role::Label => Tok::Heading,
                Role::Quantifier => Tok::Control,
                Role::Logic => Tok::Type,
                Role::Reason => Tok::Str,
                Role::MathWord => Tok::Constant,
                Role::Math => Tok::Math,
                Role::Number => Tok::Number,
                Role::Operator => Tok::Operator,
                Role::Style => Tok::Emph,
                Role::Comment => Tok::Comment,
            };
            o.push((a as u32, b as u32, t));
        }
    }
    u32::from(open)
}

fn generic(d: &LangDef, lang: Lang, s: &str, state: u32, mut out: Option<&mut Vec<Span>>) -> u32 {
    let b = s.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut push = |a: usize, e: usize, t: Tok| {
        if let Some(o) = out.as_deref_mut()
            && e > a
        {
            o.push((a as u32, e as u32, t));
        }
    };
    // carried state
    if state & STR_FLAG != 0 {
        let sdef = &d.strings[(state & 0xff) as usize];
        match string_end(s, 0, sdef) {
            Some(e) => {
                push(0, e, Tok::Str);
                i = e;
            }
            None => {
                push(0, n, Tok::Str);
                return state;
            }
        }
    } else if state > 0 && state < 256 {
        let (open, close) = d.block_comment.unwrap_or(("/*", "*/"));
        let mut depth = state;
        let mut j = 0;
        while j < n && depth > 0 {
            if s[j..].starts_with(close) {
                depth -= 1;
                j += close.len();
            } else if d.nested_comments && s[j..].starts_with(open) {
                depth += 1;
                j += open.len();
            } else {
                j += 1;
            }
        }
        push(0, j.min(n), Tok::Comment);
        if depth > 0 {
            return depth;
        }
        i = j;
    }
    let math_lang = lang == Lang::Stex;
    while i < n {
        let c = b[i];
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        if d.line_comment.iter().any(|lc| rest.starts_with(lc)) {
            push(i, n, Tok::Comment);
            return 0;
        }
        if let Some((open, close)) = d.block_comment
            && rest.starts_with(open)
        {
            let mut depth = 1u32;
            let mut j = i + open.len();
            while j < n && depth > 0 {
                if s[j..].starts_with(close) {
                    depth -= 1;
                    j += close.len();
                } else if d.nested_comments && s[j..].starts_with(open) {
                    depth += 1;
                    j += open.len();
                } else {
                    j += 1;
                }
            }
            push(i, j.min(n), Tok::Comment);
            if depth > 0 {
                return depth;
            }
            i = j;
            continue;
        }
        if d.preproc && c == b'#' && s[..i].trim().is_empty() {
            push(i, n, Tok::Macro);
            return 0;
        }
        if math_lang && c == b'$' {
            let e = s[i + 1..].find('$').map(|k| i + 1 + k + 1).unwrap_or(n);
            push(i, e, Tok::Math);
            i = e;
            continue;
        }
        let mut matched_string = false;
        for (idx, sdef) in d.strings.iter().enumerate() {
            if rest.starts_with(sdef.open) {
                // `b"` / `r"` prefixes only when not part of an identifier
                if sdef.open.len() > 1
                    && sdef.open.as_bytes()[0].is_ascii_alphabetic()
                    && i > 0
                    && is_ident(b[i - 1], false)
                {
                    continue;
                }
                match string_end(s, i + sdef.open.len(), sdef) {
                    Some(e) => {
                        push(i, e, Tok::Str);
                        i = e;
                    }
                    None if sdef.max_len != usize::MAX => {
                        // not a char literal (e.g. a Rust lifetime): highlight as a label
                        let mut j = i + 1;
                        while j < n && is_ident(b[j], false) {
                            j += 1;
                        }
                        push(i, j, Tok::Type);
                        i = j;
                    }
                    None => {
                        push(i, n, Tok::Str);
                        return if sdef.multiline { STR_FLAG | idx as u32 } else { 0 };
                    }
                }
                matched_string = true;
                break;
            }
        }
        if matched_string {
            continue;
        }
        if c.is_ascii_digit()
            || (c == b'.' && i + 1 < n && b[i + 1].is_ascii_digit() && (i == 0 || !is_ident(b[i - 1], false)))
        {
            let mut j = i + 1;
            while j < n
                && (b[j].is_ascii_alphanumeric()
                    || b[j] == b'_'
                    || (b[j] == b'.' && j + 1 < n && b[j + 1].is_ascii_digit()))
            {
                j += 1;
            }
            push(i, j, Tok::Number);
            i = j;
            continue;
        }
        if Some(c) == d.decorator && i + 1 < n && (is_ident_start(b[i + 1]) || b[i + 1] == b'[' || b[i + 1] == b'!') {
            let mut j = i + 1;
            if b[j] == b'!' {
                j += 1;
            }
            if j < n && b[j] == b'[' {
                // Rust attribute: #[...] up to the matching ]
                let mut depth = 0;
                while j < n {
                    if b[j] == b'[' {
                        depth += 1;
                    } else if b[j] == b']' {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
            } else {
                while j < n && (is_ident(b[j], d.ident_dash) || b[j] == b'.') {
                    j += 1;
                }
            }
            push(i, j, Tok::Macro);
            i = j;
            continue;
        }
        if Some(c) == d.var_sigil && i + 1 < n {
            let mut j = i + 1;
            if b[j] == b'{' {
                while j < n && b[j] != b'}' {
                    j += 1;
                }
                j = (j + 1).min(n);
            } else if b[j] == b'(' {
                while j < n && b[j] != b')' {
                    j += 1;
                }
                j = (j + 1).min(n);
            } else {
                while j < n
                    && (is_ident(b[j], false) || (j == i + 1 && (b[j].is_ascii_digit() || b"@#?*!$-".contains(&b[j]))))
                {
                    j += 1;
                    if j == i + 2 && !is_ident(b[i + 1], false) {
                        break;
                    }
                }
            }
            push(i, j, Tok::Variable);
            i = j;
            continue;
        }
        if is_ident_start(c) {
            let mut j = i + 1;
            while j < n && is_ident(b[j], d.ident_dash) {
                j += 1;
            }
            while j < n && !s.is_char_boundary(j) {
                j += 1;
            }
            let w = &s[i..j];
            let ci = d.case_insensitive;
            let tok = if d.macro_bang && j < n && b[j] == b'!' && !(j + 1 < n && b[j + 1] == b'=') {
                j += 1;
                Tok::Macro
            } else if in_list(d.control, w, ci) {
                Tok::Control
            } else if in_list(d.keywords, w, ci) {
                Tok::Keyword
            } else if in_list(d.types, w, ci) {
                Tok::Type
            } else if in_list(d.constants, w, ci) {
                Tok::Constant
            } else if s[j..].trim_start().starts_with('(') && !math_lang {
                Tok::Function
            } else if d.caps_types && w.as_bytes()[0].is_ascii_uppercase() {
                if w.len() > 1 && w.bytes().all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == b'_') {
                    Tok::Constant
                } else {
                    Tok::Type
                }
            } else if math_lang && (w.starts_with('_') || ["s.t.", "i.e."].contains(&w)) {
                Tok::Keyword
            } else {
                Tok::Text
            };
            push(i, j, tok);
            i = j;
            continue;
        }
        if b"+-*/%=<>!&|^~?:".contains(&c) {
            let mut j = i + 1;
            while j < n && b"+-*/%=<>!&|^~?:".contains(&b[j]) && !d.line_comment.iter().any(|lc| s[j..].starts_with(lc))
            {
                j += 1;
            }
            push(i, j, Tok::Operator);
            i = j;
            continue;
        }
        i += 1;
        while i < n && !s.is_char_boundary(i) {
            i += 1;
        }
    }
    0
}

fn markdown(s: &str, state: u32, mut out: Option<&mut Vec<Span>>) -> u32 {
    let n = s.len();
    let t = s.trim_start();
    let lead = n - t.len();
    let mut push = |a: usize, e: usize, k: Tok| {
        if let Some(o) = out.as_deref_mut()
            && e > a
        {
            o.push((a as u32, e as u32, k));
        }
    };
    if t.starts_with("```") || t.starts_with("~~~") {
        push(0, n, Tok::Code);
        return if state & MD_FENCE != 0 { 0 } else { MD_FENCE };
    }
    if state & MD_FENCE != 0 {
        push(0, n, Tok::Code);
        return state;
    }
    if t.starts_with('#') {
        push(0, n, Tok::Heading);
        return 0;
    }
    if t.starts_with('>') {
        push(0, n, Tok::Comment);
        return 0;
    }
    let b = s.as_bytes();
    let mut i = lead;
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        push(lead, lead + 1, Tok::Keyword);
        i = lead + 2;
    } else if let Some(p) = t.find(". ")
        && p > 0
        && p < 4
        && t[..p].bytes().all(|c| c.is_ascii_digit())
    {
        push(lead, lead + p + 1, Tok::Keyword);
        i = lead + p + 2;
    }
    while i < n {
        let c = b[i];
        let rest = &s[i..];
        if c == b'`' {
            let e = rest[1..].find('`').map(|k| i + k + 2).unwrap_or(n);
            push(i, e, Tok::Code);
            i = e;
        } else if rest.starts_with("**") || rest.starts_with("__") {
            let m = &rest[..2];
            let e = rest[2..].find(m).map(|k| i + k + 4).unwrap_or(n);
            push(i, e, Tok::Strong);
            i = e;
        } else if (c == b'*' || c == b'_') && i + 1 < n && b[i + 1] != b' ' && (i == 0 || b[i - 1] == b' ') {
            match rest[1..].find(c as char) {
                Some(k) => {
                    push(i, i + k + 2, Tok::Emph);
                    i += k + 2;
                }
                None => i += 1,
            }
        } else if c == b'$' {
            let e = rest[1..].find('$').map(|k| i + k + 2).unwrap_or(n);
            push(i, e, Tok::Math);
            i = e;
        } else if c == b'[' {
            if let Some(k) = rest.find("](") {
                let e = rest[k..].find(')').map(|q| i + k + q + 1).unwrap_or(n);
                push(i, i + k + 1, Tok::Link);
                push(i + k + 1, e, Tok::Comment);
                i = e;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
        while i < n && !s.is_char_boundary(i) {
            i += 1;
        }
    }
    0
}

fn latex(s: &str, mut state: u32, mut out: Option<&mut Vec<Span>>) -> u32 {
    let b = s.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut push = |a: usize, e: usize, k: Tok| {
        if let Some(o) = out.as_deref_mut()
            && e > a
        {
            o.push((a as u32, e as u32, k));
        }
    };
    let mut math_start = if state & TEX_DMATH != 0 { Some(0) } else { None };
    while i < n {
        let c = b[i];
        if c == b'%' && (i == 0 || b[i - 1] != b'\\') {
            if let Some(ms) = math_start {
                push(ms, i, Tok::Math);
            }
            push(i, n, Tok::Comment);
            return state;
        }
        if c == b'\\' {
            let mut j = i + 1;
            while j < n && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            if j == i + 1 && j < n {
                j += 1; // \\ , \$ , \[ ...
            }
            let cmd = &s[i..j];
            if cmd == "\\[" || cmd == "\\(" {
                math_start = Some(i);
                state |= TEX_DMATH;
                i = j;
                continue;
            }
            if cmd == "\\]" || cmd == "\\)" {
                push(math_start.unwrap_or(i), j, Tok::Math);
                math_start = None;
                state &= !TEX_DMATH;
                i = j;
                continue;
            }
            if math_start.is_none() {
                let tok = if cmd == "\\begin"
                    || cmd == "\\end"
                    || cmd.starts_with("\\section")
                    || cmd.starts_with("\\subsection")
                    || cmd == "\\documentclass"
                    || cmd == "\\usepackage"
                {
                    Tok::Control
                } else {
                    Tok::Keyword
                };
                push(i, j, tok);
                if tok == Tok::Control && j < n && b[j] == b'{' {
                    let e = s[j..].find('}').map(|k| j + k + 1).unwrap_or(n);
                    push(j, e, Tok::Type);
                    i = e;
                    continue;
                }
            }
            i = j;
            continue;
        }
        if c == b'$' {
            let dbl = i + 1 < n && b[i + 1] == b'$';
            let w = if dbl { 2 } else { 1 };
            match math_start {
                Some(ms) => {
                    push(ms, i + w, Tok::Math);
                    math_start = None;
                    state &= !TEX_DMATH;
                }
                None => {
                    math_start = Some(i);
                    if dbl {
                        state |= TEX_DMATH;
                    }
                }
            }
            i += w;
            continue;
        }
        if math_start.is_none() && (c == b'{' || c == b'}' || c == b'[' || c == b']' || c == b'&') {
            push(i, i + 1, Tok::Operator);
        }
        i += 1;
    }
    if let Some(ms) = math_start {
        push(ms, n, Tok::Math);
        if state & TEX_DMATH == 0 {
            // inline $...$ doesn't span lines
        }
    }
    state
}

/// Per-document cache of line-start states so highlighting is incremental.
#[derive(Default)]
pub struct HlCache {
    states: Vec<u32>,
    /// `states[i]` is valid for all `i < valid`.
    valid: usize,
}

impl HlCache {
    pub fn invalidate_from(&mut self, line: usize) {
        self.valid = self.valid.min(line);
    }

    /// State at the start of `line`, computing forward from the last valid line.
    pub fn state_at(&mut self, lang: Lang, lines: &[String], line: usize) -> u32 {
        if lang == Lang::Plain {
            return 0;
        }
        if self.states.len() < lines.len() + 1 {
            self.states.resize(lines.len() + 1, 0);
        }
        if self.valid == 0 {
            self.states[0] = 0;
            self.valid = 1;
        }
        let target = line.min(lines.len());
        while self.valid <= target {
            let i = self.valid - 1;
            let st = self.states[i];
            self.states[i + 1] = highlight_line(lang, &lines[i], st, None);
            self.valid += 1;
        }
        self.states[target]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(lang: Lang, s: &str) -> Vec<(String, Tok)> {
        let mut v = Vec::new();
        highlight_line(lang, s, 0, Some(&mut v));
        v.into_iter().map(|(a, b, t)| (s[a as usize..b as usize].to_string(), t)).collect()
    }

    #[test]
    fn rust_basics() {
        let t = toks(Lang::Rust, "fn main() { let x = \"hi\"; println!(\"{}\", 42); } // c");
        assert!(t.contains(&("fn".into(), Tok::Keyword)));
        assert!(t.contains(&("main".into(), Tok::Function)));
        assert!(t.contains(&("\"hi\"".into(), Tok::Str)));
        assert!(t.contains(&("println!".into(), Tok::Macro)));
        assert!(t.contains(&("42".into(), Tok::Number)));
        assert!(t.contains(&("// c".into(), Tok::Comment)));
        let t = toks(Lang::Rust, "fn f<'a>(x: &'a str) -> char { 'x' }");
        assert!(t.contains(&("'a".into(), Tok::Type)));
        assert!(t.contains(&("'x'".into(), Tok::Str)));
    }

    #[test]
    fn block_comment_state() {
        let st = highlight_line(Lang::C, "int x; /* start", 0, None);
        assert!(st > 0);
        let st2 = highlight_line(Lang::C, "still */ int y;", st, None);
        assert_eq!(st2, 0);
        let st = highlight_line(Lang::Python, "x = \"\"\"doc", 0, None);
        assert!(st & STR_FLAG != 0);
        assert_eq!(highlight_line(Lang::Python, "end\"\"\"", st, None), 0);
    }

    #[test]
    fn markdown_and_latex() {
        let t = toks(Lang::Markdown, "# Title");
        assert_eq!(t, vec![("# Title".into(), Tok::Heading)]);
        let t = toks(Lang::Latex, "\\begin{proof} $x^2$ % c");
        assert!(t.contains(&("\\begin".into(), Tok::Control)));
        assert!(t.contains(&("{proof}".into(), Tok::Type)));
        assert!(t.contains(&("$x^2$".into(), Tok::Math)));
        assert!(t.contains(&("% c".into(), Tok::Comment)));
    }
}
