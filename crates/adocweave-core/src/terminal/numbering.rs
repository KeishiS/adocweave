//! Ordered-list numbers written out for a terminal.
//!
//! HTML leaves the shape of a number to a stylesheet. A terminal has none, so
//! the number itself is written here, in the style the document asked for.

use crate::block_model::{ListBlock, OrderedListPresentation, OrderedListStyle};

/// The lowercase Greek letters AsciiDoc uses, in order.
const GREEK: [char; 24] = [
    'α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ', 'ν', 'ξ', 'ο', 'π', 'ρ', 'σ', 'τ',
    'υ', 'φ', 'χ', 'ψ', 'ω',
];

const ROMAN: [(u32, &str); 13] = [
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

/// The number of every item of `list`, in reading order.
///
/// The count starts where the document says it does, runs backwards for a
/// reversed list, and continues from any number an item states for itself.
pub(super) fn item_numbers(list: &ListBlock) -> Vec<u32> {
    let presentation = list.presentation;
    let mut next = start_of(&presentation, list.items.len());
    let mut numbers = Vec::with_capacity(list.items.len());
    for item in &list.items {
        let number = item.explicit_number.unwrap_or(next);
        numbers.push(number);
        next = if presentation.reversed {
            number.saturating_sub(1)
        } else {
            number.saturating_add(1)
        };
    }
    numbers
}

fn start_of(presentation: &OrderedListPresentation, items: usize) -> u32 {
    match presentation.start {
        Some(start) => start,
        // A reversed list without a stated start counts down to one.
        None if presentation.reversed => u32::try_from(items).unwrap_or(u32::MAX).max(1),
        None => 1,
    }
}

/// `number` written in `style`, without the separator that follows it.
pub(super) fn label(number: u32, style: OrderedListStyle) -> String {
    match style {
        OrderedListStyle::Arabic | OrderedListStyle::Decimal => number.to_string(),
        OrderedListStyle::LowerAlpha => alphabetic(number, 'a'),
        OrderedListStyle::UpperAlpha => alphabetic(number, 'A'),
        OrderedListStyle::LowerRoman => roman(number),
        OrderedListStyle::UpperRoman => roman(number).to_uppercase(),
        OrderedListStyle::LowerGreek => greek(number),
    }
}

/// `a` to `z`, then `aa`, `ab`, and so on, so a list longer than the alphabet
/// still numbers every item differently.
fn alphabetic(number: u32, first: char) -> String {
    if number == 0 {
        return first.to_string();
    }
    let mut value = number;
    let mut letters = Vec::new();
    while value > 0 {
        let index = (value - 1) % 26;
        letters.push(
            char::from_u32(first as u32 + index).expect("a letter of the Latin alphabet exists"),
        );
        value = (value - 1) / 26;
    }
    letters.iter().rev().collect()
}

fn greek(number: u32) -> String {
    if number == 0 {
        return GREEK[0].to_string();
    }
    let mut value = number;
    let mut letters = Vec::new();
    let count = u32::try_from(GREEK.len()).expect("the alphabet is short");
    while value > 0 {
        let index = usize::try_from((value - 1) % count).expect("an index into the alphabet");
        letters.push(GREEK[index]);
        value = (value - 1) / count;
    }
    letters.iter().rev().collect()
}

/// Roman numerals, in lowercase. A number too large to write, or zero, keeps
/// its digits rather than turning into something unreadable.
fn roman(number: u32) -> String {
    if number == 0 || number > 3999 {
        return number.to_string();
    }
    let mut value = number;
    let mut text = String::new();
    for (amount, numeral) in ROMAN {
        while value >= amount {
            text.push_str(numeral);
            value -= amount;
        }
    }
    text
}
