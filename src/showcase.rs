//! The reference note: one document exercising every markdown construct the
//! renderer understands, and naming the ones it does not.
//!
//! It lives here rather than in the seeding code because it is two things at
//! once — the note a reader opens to see what the two views do, and the
//! fixture the parser is tested against. Both want the same text, and a
//! feature added to one without the other is how a showcase goes stale.

/// The reference document, installed into a vault that does not have it.
pub const SHOWCASE: &str = r##"Markdown showcase
=================

Everything below is written in the source tab and drawn in the preview tab.
Switch between them with `cmd-1` and `cmd-2`, with the tabs above, or with
`:source` and `:preview`.

## Headings

Six levels are parsed. There is one font at one size, so the ladder is drawn
with everything else: the first three rule themselves off, the last one gives
up its weight, and the colour and the air above step down the whole way. This
heading and the title above show both ways of writing one.

### Third level
#### Fourth level
##### Fifth level
###### Sixth level
### Closed with hashes ###

## Emphasis

Text can be *italic* or _italic_, **bold** or __bold__, `monospaced`,
~~struck out~~, or ***all at once***. Emphasis nests, so **bold with an
*italic* word inside** keeps both, and `**backticks**` win over everything
inside them.

Delimiters can be escaped: \*not italic\*, \_not italic\_, \`not code\`, and
a stray backslash in C:\path\to\file stays where it is.

A code span with a backtick in it is fenced by two: `` a ` b ``. Underscores
in snake_case_identifiers are left alone, and 2 * 3 * 4 is arithmetic.

## Links

An inline [link](https://example.com), one [with a title](https://example.com "hover me"),
and one whose [destination is bracketed](<https://example.com/with space>).

An autolink stands on its own: <https://example.com/autolink>. So does an
address: <someone@example.com>. A bare URL is picked up where it stands -
https://example.com/bare - and so is www.example.com, and the full stop
ending this sentence is not part of it: https://example.com/end.

Reference links come in three shapes: [a full one][ref], [a collapsed one][],
and [a shortcut]. Their targets live at the bottom of this file and are not
drawn.

Links can be emphasised too: [**a loud link**](https://example.com).

An image cannot be drawn, so its alt text stands in for it:
![a picture of a cat](cat.png)

Links to other notes work: [the welcome note](welcome.md) opens it.

## Paragraphs and breaks

A paragraph is not a line. These three source lines
are one paragraph, and they reflow to whatever
width the pane happens to be.

Two trailing spaces ask for a break,  
so this starts a new line inside the same paragraph.
A trailing backslash does the same,\
like this.

A blank line starts a new paragraph.

## Lists

- A bullet at the top level
* The same, with an asterisk
+ And with a plus
  - A nested bullet, one indent deeper
  - And its sibling
- A long item that runs onto a second source line
  and continues here without starting anything new
- Back out again

1. Ordered items keep their numbers
2. Even when they are not sequential
7. As here
1) A closing paren works as well

- [ ] An unchecked task
- [x] A checked one
- [X] Capital X counts too

A blank line between items does not split the list:

- one

- two

- three

## Quotes

> A quote is a place, not a style.
> A soft wrap inside one is still a soft wrap.

A quote may run on without repeating the marker:

> This starts a quote
and this line continues it lazily.

Anything a document can hold, a quote can hold:

> ### A heading inside a quote
>
> - with a list
> - under it
>
> ```rust
> fn quoted() -> &'static str { "and code" }
> ```
>
> > and a quote inside the quote

## Code

A fenced block with a language gets real syntax highlighting:

```rust
fn main() {
    let note = "every note is just a file on disk";
    println!("{note}");
}
```

A tilde fence can hold a backtick fence without ending early:

~~~
```
this stays inside
```
~~~

A longer fence closes only on one at least as long:

````
```
still inside
```
````

Four spaces of indent is a code block too:

    fn indented() {}
    // with no language

## Tables

Columns size to their content, and the alignment row decides where each one
sits:

| Left | Centre | Right |
|:-----|:------:|------:|
| a | b | c |
| a longer cell | middle | 42 |
| `code` | **bold** | [a link](https://example.com) |

## Rules

Three ways to write a horizontal rule:

---

***

___

## Not supported

These are left as plain text rather than pretended at:

- Inline and block HTML
- Footnotes and definition lists
- Character entities like `&amp;`
- Any script the 5x7 ASCII font cannot draw

[ref]: https://example.com/full "a title nobody can hover"
[a collapsed one]: https://example.com/collapsed
[a shortcut]: https://example.com/shortcut
"##;
