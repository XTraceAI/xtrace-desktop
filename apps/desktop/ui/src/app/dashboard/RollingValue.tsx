import type { CSSProperties } from 'react';

/**
 * One character of a formatted value, keyed by its place: integer digits and
 * group separators count from the decimal point leftwards, fraction digits
 * rightwards, so a digit keeps its element — and rolls — only while it stays
 * the same place value. "9.8" becoming "10.2" rolls the units and the tenths
 * and adds the tens; "12" becoming "12.4" adds the point and the tenth without
 * moving the 1 or the 2.
 */
export function places(text: string): { key: string; char: string }[] {
  const point = text.indexOf('.');
  const end = point === -1 ? text.length : point;
  return [...text].map((char, index) => {
    const digit = /\d/.test(char);
    if (index > end) return { key: `f${index - end}${digit ? 'd' : char}`, char };
    if (index === end) return { key: 'point', char };
    // Integer side: its place, counted in digits from the point.
    const place = [...text.slice(index + 1, end)].filter((next) => /\d/.test(next)).length;
    return { key: digit ? `i${place}` : `i${place}${char}`, char };
  });
}

/**
 * A formatted value whose digits roll to their new figure when the value
 * changes. The value is read once, whole, from one visually hidden string —
 * exactly the text the formatter gave — so assistive technology, copying and
 * the page's text only ever meet the current value, never a figure on its way
 * or a digit at a time. The figures on screen are drawn beside it by CSS alone
 * (`dashboard.css`) from each character's data, under `aria-hidden`: a digit
 * keeps its place's element, and its column of figures transitions to the new
 * one, so a first value, a digit that only now appears and a reduced-motion
 * preference all show the figure at once, and a value that changes again
 * mid-roll retargets the same transition — no timer to clear, no intermediate
 * number the report never stated.
 */
export function RollingValue({ text }: { text: string }) {
  return (
    <span className="xt-roll" data-testid="rolling-value">
      <span className="sr-only">{text}</span>
      <span className="xt-roll-face" aria-hidden="true">
        {places(text).map(({ key, char }) =>
          /\d/.test(char) ? (
            <span
              key={key}
              className="xt-roll-digit"
              data-digit={char}
              style={{ '--digit': char } as CSSProperties}
            />
          ) : (
            <span key={key} className="xt-roll-char" data-char={char} />
          ),
        )}
      </span>
    </span>
  );
}
