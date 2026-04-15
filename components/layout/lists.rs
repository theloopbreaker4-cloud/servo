/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use style::counter_style::{CounterStyle, Symbol, SymbolsType};
use style::properties::longhands::list_style_type::computed_value::T as ListStyleType;
use style::values::computed::Image;
use style::values::generics::counters::Content;
use stylo_atoms::atom;

use crate::context::LayoutContext;
use crate::dom_traversal::{
    NodeAndStyleInfo, PseudoElementContentItem, generate_pseudo_element_content,
};
use crate::replaced::ReplacedContents;

/// <https://drafts.csswg.org/css-lists/#content-property>
pub(crate) fn make_marker<'dom>(
    context: &LayoutContext,
    info: &NodeAndStyleInfo<'dom>,
) -> Option<(NodeAndStyleInfo<'dom>, Vec<PseudoElementContentItem>)> {
    let marker_info =
        info.with_pseudo_element(context, style::selector_parser::PseudoElement::Marker)?;
    let style = &marker_info.style;
    let list_style = style.get_list();

    // https://drafts.csswg.org/css-lists/#marker-image
    let marker_image = || match &list_style.list_style_image {
        Image::Url(url) => Some(vec![
            PseudoElementContentItem::Replaced(ReplacedContents::from_image_url(
                marker_info.node,
                context,
                url,
            )?),
            PseudoElementContentItem::Text(" ".into()),
        ]),
        // XXX: Non-None image types unimplemented.
        Image::ImageSet(..) |
        Image::Gradient(..) |
        Image::CrossFade(..) |
        Image::PaintWorklet(..) |
        Image::None => None,
        Image::LightDark(..) => unreachable!("light-dark() should be disabled"),
    };

    let content = match &marker_info.style.get_counters().content {
        Content::Items(_) => generate_pseudo_element_content(&marker_info, context),
        Content::None => return None,
        Content::Normal => marker_image().or_else(|| {
            Some(vec![PseudoElementContentItem::Text(marker_string(
                &list_style.list_style_type,
            )?)])
        })?,
    };

    Some((marker_info, content))
}

fn symbol_to_string(symbol: &Symbol) -> &str {
    match symbol {
        Symbol::String(string) => string,
        Symbol::Ident(ident) => &ident.0,
    }
}

/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>
/// Generate counter representation for a specific integer value.
pub(crate) fn generate_counter_value(value: i32, counter_style: &CounterStyle) -> String {
    match counter_style {
        CounterStyle::None | CounterStyle::String(_) => unreachable!("Invalid counter style"),
        CounterStyle::Name(name) => match name.0 {
            // Symbol-based styles (don't depend on numeric value).
            atom!("disc") => "\u{2022}".to_string(),
            atom!("circle") => "\u{25E6}".to_string(),
            atom!("square") => "\u{25AA}".to_string(),
            atom!("disclosure-open") => "\u{25BE}".to_string(),
            atom!("disclosure-closed") => "\u{25B8}".to_string(),
            atom!("none") => String::new(),

            // Numeric styles — these depend on the counter value.
            atom!("decimal") => value.to_string(),
            atom!("decimal-leading-zero") => {
                if value.abs() < 10 {
                    if value < 0 {
                        format!("-0{}", -value)
                    } else {
                        format!("0{value}")
                    }
                } else {
                    value.to_string()
                }
            },
            atom!("lower-roman") => int_to_roman(value, false),
            atom!("upper-roman") => int_to_roman(value, true),
            atom!("lower-alpha") | atom!("lower-latin") => int_to_alpha(value, false),
            atom!("upper-alpha") | atom!("upper-latin") => int_to_alpha(value, true),
            atom!("lower-greek") => int_to_greek(value),
            atom!("arabic-indic") => int_to_digits(value, "\u{660}\u{661}\u{662}\u{663}\u{664}\u{665}\u{666}\u{667}\u{668}\u{669}"),
            atom!("devanagari") => int_to_digits(value, "\u{966}\u{967}\u{968}\u{969}\u{96A}\u{96B}\u{96C}\u{96D}\u{96E}\u{96F}"),
            atom!("thai") => int_to_digits(value, "\u{E50}\u{E51}\u{E52}\u{E53}\u{E54}\u{E55}\u{E56}\u{E57}\u{E58}\u{E59}"),
            atom!("persian") => int_to_digits(value, "\u{6F0}\u{6F1}\u{6F2}\u{6F3}\u{6F4}\u{6F5}\u{6F6}\u{6F7}\u{6F8}\u{6F9}"),

            // Styles not registered as atoms — compare by string.
            ref a if a.as_ref() == "lower-hexadecimal" => {
                if value < 0 { format!("-{:x}", -value) } else { format!("{value:x}") }
            },
            ref a if a.as_ref() == "upper-hexadecimal" => {
                if value < 0 { format!("-{:X}", -value) } else { format!("{value:X}") }
            },
            ref a if a.as_ref() == "octal" => {
                if value < 0 { format!("-{:o}", -value) } else { format!("{value:o}") }
            },
            ref a if a.as_ref() == "binary" => {
                if value < 0 { format!("-{:b}", -value) } else { format!("{value:b}") }
            },

            // Everything else: fall back to decimal.
            _ => value.to_string(),
        },
        CounterStyle::Symbols { ty, symbols } => {
            let syms: Vec<&str> = symbols.0.iter().map(|s| symbol_to_string(s)).collect();
            if syms.is_empty() {
                return value.to_string();
            }
            match ty {
                SymbolsType::Cyclic => {
                    let n = syms.len();
                    let idx = if value > 0 {
                        ((value - 1) as usize) % n
                    } else {
                        // For non-positive: fall back to first symbol
                        0
                    };
                    syms[idx].to_string()
                },
                SymbolsType::Numeric => {
                    // Numeric: treat symbols as digits in base-N.
                    let n = syms.len() as i32;
                    if n < 2 { return value.to_string(); }
                    if value <= 0 { return syms[0].to_string(); }
                    let mut v = value;
                    let mut result = Vec::new();
                    while v > 0 {
                        result.push(syms[(v % n) as usize]);
                        v /= n;
                    }
                    result.iter().rev().cloned().collect::<Vec<_>>().join("")
                },
                SymbolsType::Alphabetic => {
                    // Alphabetic: 1-based. Similar to alpha but with custom symbols.
                    if value <= 0 { return value.to_string(); }
                    let n = syms.len() as i32;
                    let mut v = value;
                    let mut result = Vec::new();
                    while v > 0 {
                        v -= 1;
                        result.push(syms[(v % n) as usize]);
                        v /= n;
                    }
                    result.iter().rev().cloned().collect::<Vec<_>>().join("")
                },
                SymbolsType::Fixed => {
                    // Fixed: maps value to symbol directly (1-indexed).
                    let idx = (value - 1) as usize;
                    if idx < syms.len() {
                        syms[idx].to_string()
                    } else {
                        value.to_string()
                    }
                },
                SymbolsType::Symbolic => {
                    // Symbolic: repeats symbol N times where N = ceil(value / len).
                    if value <= 0 { return value.to_string(); }
                    let n = syms.len() as i32;
                    let sym_idx = ((value - 1) % n) as usize;
                    let repeat = ((value - 1) / n + 1) as usize;
                    syms[sym_idx].repeat(repeat)
                },
            }
        },
    }
}

/// Convert integer to alphabetic counter (a, b, c, ... z, aa, ab, ...).
fn int_to_alpha(mut value: i32, upper: bool) -> String {
    if value <= 0 {
        return value.to_string();
    }
    let base = if upper { b'A' } else { b'a' };
    let mut result = Vec::new();
    while value > 0 {
        value -= 1;
        result.push((base + (value % 26) as u8) as char);
        value /= 26;
    }
    result.iter().rev().collect()
}

/// Convert integer to Greek lowercase (α, β, γ, ...).
fn int_to_greek(value: i32) -> String {
    const GREEK: &[char] = &[
        'α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ',
        'ν', 'ξ', 'ο', 'π', 'ρ', 'σ', 'τ', 'υ', 'φ', 'χ', 'ψ', 'ω',
    ];
    if value <= 0 || value > GREEK.len() as i32 {
        return value.to_string();
    }
    GREEK[(value - 1) as usize].to_string()
}

/// Convert integer to digits using a custom digit string (like Arabic-Indic).
fn int_to_digits(value: i32, digit_chars: &str) -> String {
    let digits: Vec<char> = digit_chars.chars().collect();
    if digits.len() < 10 { return value.to_string(); }
    if value == 0 { return digits[0].to_string(); }
    let negative = value < 0;
    let mut v = value.unsigned_abs();
    let mut result = Vec::new();
    while v > 0 {
        result.push(digits[(v % 10) as usize]);
        v /= 10;
    }
    if negative { result.push('-'); }
    result.iter().rev().collect()
}

/// Convert integer to Roman numerals.
fn int_to_roman(value: i32, upper: bool) -> String {
    if value <= 0 || value >= 4000 {
        return value.to_string();
    }
    const VALS: &[(u32, &str, &str)] = &[
        (1000, "M", "m"), (900, "CM", "cm"), (500, "D", "d"), (400, "CD", "cd"),
        (100, "C", "c"), (90, "XC", "xc"), (50, "L", "l"), (40, "XL", "xl"),
        (10, "X", "x"), (9, "IX", "ix"), (5, "V", "v"), (4, "IV", "iv"),
        (1, "I", "i"),
    ];
    let mut v = value as u32;
    let mut result = String::new();
    for &(n, up, lo) in VALS {
        while v >= n {
            result.push_str(if upper { up } else { lo });
            v -= n;
        }
    }
    result
}

/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>
/// Legacy version — returns &str, assumes value=0.
/// Kept for compatibility with marker rendering (which doesn't use counters yet).
pub(crate) fn generate_counter_representation(counter_style: &CounterStyle) -> &str {
    match counter_style {
        CounterStyle::None | CounterStyle::String(_) => unreachable!("Invalid counter style"),
        CounterStyle::Name(name) => match name.0 {
            atom!("disc") => "\u{2022}",
            atom!("circle") => "\u{25E6}",
            atom!("square") => "\u{25AA}",
            atom!("disclosure-open") => "\u{25BE}",
            atom!("disclosure-closed") => "\u{25B8}",
            atom!("decimal-leading-zero") => "00",
            atom!("arabic-indic") => "\u{660}",
            atom!("bengali") => "\u{9E6}",
            atom!("cambodian") | atom!("khmer") => "\u{17E0}",
            atom!("devanagari") => "\u{966}",
            atom!("gujarati") => "\u{AE6}",
            atom!("gurmukhi") => "\u{A66}",
            atom!("kannada") => "\u{CE6}",
            atom!("lao") => "\u{ED0}",
            atom!("malayalam") => "\u{D66}",
            atom!("mongolian") => "\u{1810}",
            atom!("myanmar") => "\u{1040}",
            atom!("oriya") => "\u{B66}",
            atom!("persian") => "\u{6F0}",
            atom!("tamil") => "\u{BE6}",
            atom!("telugu") => "\u{C66}",
            atom!("thai") => "\u{E50}",
            atom!("tibetan") => "\u{F20}",
            atom!("cjk-decimal") |
            atom!("cjk-earthly-branch") |
            atom!("cjk-heavenly-stem") |
            atom!("japanese-informal") => "\u{3007}",
            atom!("korean-hangul-formal") => "\u{C601}",
            atom!("korean-hanja-informal") |
            atom!("korean-hanja-formal") |
            atom!("japanese-formal") |
            atom!("simp-chinese-informal") |
            atom!("simp-chinese-formal") |
            atom!("trad-chinese-informal") |
            atom!("trad-chinese-formal") |
            atom!("cjk-ideographic") => "\u{96F6}",
            _ => "0",
        },
        CounterStyle::Symbols { ty, symbols } => match ty {
            SymbolsType::Numeric => {
                symbol_to_string(symbols.0.first().expect("symbols() should have symbols"))
            },
            SymbolsType::Cyclic => {
                symbol_to_string(symbols.0.last().expect("symbols() should have symbols"))
            },
            SymbolsType::Alphabetic | SymbolsType::Symbolic | SymbolsType::Fixed => "0",
        },
    }
}

/// <https://drafts.csswg.org/css-lists/#marker-string>
pub(crate) fn marker_string(list_style_type: &ListStyleType) -> Option<String> {
    let suffix = match &list_style_type.0 {
        CounterStyle::None => return None,
        CounterStyle::String(string) => return Some(string.to_string()),
        CounterStyle::Name(name) => match name.0 {
            atom!("disc") |
            atom!("circle") |
            atom!("square") |
            atom!("disclosure-open") |
            atom!("disclosure-closed") => " ",
            atom!("hiragana") |
            atom!("hiragana-iroha") |
            atom!("katakana") |
            atom!("katakana-iroha") |
            atom!("cjk-decimal") |
            atom!("cjk-earthly-branch") |
            atom!("cjk-heavenly-stem") |
            atom!("japanese-informal") |
            atom!("japanese-formal") |
            atom!("simp-chinese-informal") |
            atom!("simp-chinese-formal") |
            atom!("trad-chinese-informal") |
            atom!("trad-chinese-formal") |
            atom!("cjk-ideographic") => "\u{3001}", /* "、" */
            atom!("korean-hangul-formal") |
            atom!("korean-hanja-informal") |
            atom!("korean-hanja-formal") => ", ",
            atom!("ethiopic-numeric") => "/ ",
            _ => ". ",
        },
        CounterStyle::Symbols { .. } => " ",
    };
    Some(generate_counter_representation(&list_style_type.0).to_string() + suffix)
}
