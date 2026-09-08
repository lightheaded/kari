import Compile, { type MarkdownToJSX } from "markdown-to-jsx";

/**
 * The text here comes out of a transcript, so it is not trusted. Nothing is ever
 * handed to `dangerouslySetInnerHTML`: the compiler builds React elements, and
 * both HTML doors are shut, so a reply that quotes a tag shows the tag as text.
 * The compiler's own sanitizer still drops `javascript:` and friends from any
 * link the markdown carries.
 */
const OPTIONS: MarkdownToJSX.Options = {
  disableParsingRawHTML: true,
  ignoreHTMLBlocks: true,
  // A one-line reply is still a paragraph, and the box needs an element to hang
  // its class on either way. Without these a short answer renders bare and
  // loses the spacing the rest of a reply gets.
  forceBlock: true,
  forceWrapper: true,
  wrapper: "div",
  // Same treatment the PR links get: a click leaves for the browser rather than
  // navigating the app's own webview away from the board.
  overrides: { a: { props: { target: "_blank", rel: "noreferrer" } } },
};

/** Renders markdown from a transcript. `className` styles the scroll box. */
export function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <Compile className={className} options={OPTIONS}>
      {text}
    </Compile>
  );
}
