//! Document assembly: body rendering, the layout pass (spec feature 8) and package detection
//! (spec feature 9). STEX-DESIGN §3.7, §7.

use crate::proof::{Block, Out};

/// Render blocks as LaTeX body text: one sentence per source line, blank line between paragraphs.
pub fn render(blocks: &[Block]) -> String {
    let mut s = String::new();
    for (k, b) in blocks.iter().enumerate() {
        if k > 0 {
            s.push_str("\n\n");
        }
        match b {
            Block::Para(outs) => render_outs(outs, "", &mut s),
            Block::Items(items) => {
                s.push_str("\\begin{itemize}\n");
                for it in items {
                    s.push_str("  \\item ");
                    render_outs(it, "    ", &mut s);
                    s.push('\n');
                }
                s.push_str("\\end{itemize}");
            }
        }
    }
    s
}

fn render_outs(outs: &[Out], indent: &str, s: &mut String) {
    let mut first = true;
    // a blank line inside a list item (a new paragraph within one case)
    let mut gap = false;
    for o in outs {
        if matches!(o, Out::Break) {
            gap = !first;
            continue;
        }
        if !first {
            s.push('\n');
            if gap {
                s.push('\n');
            }
            s.push_str(indent);
        }
        first = false;
        gap = false;
        match o {
            Out::Break => {}
            Out::Sentence(t) => s.push_str(t),
            Out::Display(d) => s.push_str(&d.replace('\n', &format!("\n{indent}"))),
            Out::List(items) => {
                // nested itemize (sub-cases, steps inside a case)
                s.push_str("\\begin{itemize}");
                for it in items {
                    s.push('\n');
                    s.push_str(indent);
                    s.push_str("  \\item ");
                    render_outs(it, &format!("{indent}    "), s);
                }
                s.push('\n');
                s.push_str(indent);
                s.push_str("\\end{itemize}");
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub margin: &'static str,
    pub ragged_bottom: bool,
    pub allow_display_breaks: bool,
    pub needspace: bool,
    pub penalties: bool,
    /// Estimated length in pages (for diagnostics/tests).
    pub pages: f32,
}

const LINES_PER_PAGE: f32 = 46.0;
const CHARS_PER_LINE: f32 = 92.0;

/// Visible characters of a LaTeX string (commands and braces dropped).
fn visible(s: &str) -> usize {
    let mut n = 0;
    let mut cmd = false;
    for c in s.chars() {
        if c == '\\' {
            cmd = true;
            continue;
        }
        if cmd {
            if c.is_ascii_alphabetic() {
                continue;
            }
            cmd = false;
        }
        if !matches!(c, '{' | '}' | '$' | '^' | '_' | '&') {
            n += 1;
        }
    }
    n
}

fn outs_lines(outs: &[Out]) -> (f32, usize) {
    let mut lines = 0.0;
    let mut text = 0usize;
    for o in outs {
        match o {
            Out::Sentence(t) => text += visible(t) + 1,
            Out::Break => {
                lines += (text as f32 / CHARS_PER_LINE).ceil();
                text = 0;
            }
            Out::List(items) => {
                lines += (text as f32 / CHARS_PER_LINE).ceil() + 0.5;
                text = 0;
                for it in items {
                    lines += outs_lines(it).0 + 0.3;
                }
            }
            Out::Display(d) => {
                lines += (text as f32 / CHARS_PER_LINE).ceil();
                text = 0;
                lines += if d.starts_with("\\begin{align") { d.matches("\\\\").count() as f32 + 2.0 } else { 2.5 };
            }
        }
    }
    lines += (text as f32 / CHARS_PER_LINE).ceil();
    let labels = outs.iter().filter(|o| matches!(o, Out::Sentence(t) if t.starts_with("\\textit{"))).count();
    (lines, labels)
}

/// Page-fit heuristics: avoid a nearly-empty last page, stranded headers, split short sections and
/// stretched whitespace. Deterministic and cheap (character-count estimates).
pub fn layout(blocks: &[Block]) -> Plan {
    let mut lines = 0.0;
    let mut labels = 0;
    let mut has_align = false;
    for b in blocks {
        match b {
            Block::Para(o) => {
                let (l, n) = outs_lines(o);
                lines += l + 0.6;
                labels += n;
                has_align |= o.iter().any(|x| matches!(x, Out::Display(d) if d.starts_with("\\begin{align")));
            }
            Block::Items(items) => {
                for it in items {
                    let (l, n) = outs_lines(it);
                    lines += l + 0.3;
                    labels += n;
                    has_align |= it.iter().any(|x| matches!(x, Out::Display(d) if d.starts_with("\\begin{align")));
                }
                lines += 1.0;
            }
        }
    }
    let pages = lines / LINES_PER_PAGE;
    let mut plan = Plan {
        margin: "1in",
        ragged_bottom: false,
        allow_display_breaks: false,
        needspace: false,
        penalties: false,
        pages,
    };
    if pages <= 1.0 {
        return plan;
    }
    if pages <= 1.12 {
        // just over one page: tighter margins pull it back onto a single page
        plan.margin = "0.75in";
        return plan;
    }
    // multi-page documents
    plan.ragged_bottom = true;
    plan.penalties = true;
    let avg_section = if labels > 0 { lines / labels as f32 } else { lines };
    if labels >= 2 && avg_section < 0.5 * LINES_PER_PAGE {
        // many short sections: keep each header with its first lines
        plan.needspace = true;
    }
    if has_align && (labels <= 1 || avg_section >= 0.5 * LINES_PER_PAGE) {
        // one long section: let long derivations break across pages instead of leaving gaps
        plan.allow_display_breaks = true;
    }
    plan
}

/// Packages needed by the body (spec feature 9), in a stable order.
pub fn packages(body: &str, plan: &Plan) -> Vec<&'static str> {
    let has = |needles: &[&str]| needles.iter().any(|n| body.contains(n));
    let mut p = Vec::new();
    if body.contains('$') || body.contains("\\[") || body.contains("\\begin{align") || body.contains("\\begin{equation")
    {
        p.push("amsmath");
    }
    if has(&[
        "\\mathbb",
        "\\varnothing",
        "\\nmid",
        "\\square",
        "\\mathfrak",
        "\\nexists",
        "\\ncong",
        "\\nsim",
        "\\subsetneq",
        "\\therefore",
        "\\blacksquare",
        "\\lesssim",
        "\\gtrsim",
        "\\leqslant",
        "\\geqslant",
        "\\complement",
    ]) {
        p.push("amssymb");
    }
    if has(&["\\mathscr"]) {
        p.push("mathrsfs");
    }
    if has(&["\\begin{theorem", "\\begin{lemma", "\\begin{proof", "\\qedhere"]) {
        p.push("amsthm");
    }
    if has(&["\\begin{enumerate}[", "\\begin{itemize}["]) {
        p.push("enumitem");
    }
    if has(&["\\includegraphics"]) {
        p.push("graphicx");
    }
    if has(&["\\begin{tikzpicture}", "\\tikz"]) {
        p.push("tikz");
    }
    if has(&["\\color{", "\\textcolor"]) {
        p.push("xcolor");
    }
    if plan.needspace {
        p.push("needspace");
    }
    if has(&["\\href", "\\url{"]) {
        p.push("hyperref");
    }
    p
}

/// Insert `\needspace` before labelled sections (only when the plan asks for it).
fn apply_needspace(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 64);
    for (k, para) in body.split("\n\n").enumerate() {
        if k > 0 {
            out.push_str("\n\n");
        }
        if para.starts_with("\\textit{") && k > 0 {
            out.push_str("\\needspace{6\\baselineskip}\n");
        }
        out.push_str(para);
    }
    out
}

pub fn assemble(body: &str, plan: &Plan, source_name: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("% Generated by `cmx stex compile` from {source_name}. Edit the .stex file, not this one.\n"));
    s.push_str("\\documentclass[11pt]{article}\n");
    // Latin Modern (Type 1) in T1 encoding: every glyph has an outline font, so the bundled TinyTeX
    // never needs to generate bitmap fonts (it ships no METAFONT sources for the EC fonts).
    s.push_str("\\usepackage[T1]{fontenc}\n\\usepackage{lmodern}\n");
    s.push_str(&format!("\\usepackage[margin={}]{{geometry}}\n", plan.margin));
    for p in packages(body, plan) {
        s.push_str(&format!("\\usepackage{{{p}}}\n"));
    }
    if plan.ragged_bottom {
        s.push_str("\\raggedbottom\n");
    }
    if plan.penalties {
        s.push_str("\\widowpenalty=10000\n\\clubpenalty=10000\n");
    }
    if plan.allow_display_breaks {
        s.push_str("\\allowdisplaybreaks\n");
    }
    s.push_str("\\begin{document}\n\n");
    if plan.needspace {
        s.push_str(&apply_needspace(body));
    } else {
        s.push_str(body);
    }
    s.push_str("\n\n\\end{document}\n");
    s
}
