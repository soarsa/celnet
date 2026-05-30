/**
 * A tiny, dependency-free fuzzy matcher for the command palette (GUI-DESIGN
 * §2 navigation grammar). Subsequence match with a contiguity/word-boundary
 * bonus; returns a score and the matched character indices for highlighting.
 */

export interface FuzzyMatch {
  score: number;
  indices: number[];
}

/** Match `query` against `text` (case-insensitive). Returns null on no match. */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  if (query.length === 0) return { score: 0, indices: [] };
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  const indices: number[] = [];
  let score = 0;
  let ti = 0;
  let lastMatch = -2;
  for (let qi = 0; qi < q.length; qi += 1) {
    const ch = q[qi]!;
    let found = -1;
    for (; ti < t.length; ti += 1) {
      if (t[ti] === ch) {
        found = ti;
        break;
      }
    }
    if (found === -1) return null;
    // Contiguity bonus.
    if (found === lastMatch + 1) score += 6;
    // Word-boundary bonus.
    const prev = found > 0 ? t[found - 1]! : " ";
    if (prev === " " || prev === "/" || prev === "-") score += 4;
    // Start-of-string bonus.
    if (found === 0) score += 5;
    score += 1;
    indices.push(found);
    lastMatch = found;
    ti = found + 1;
  }
  // Brevity bonus: shorter targets that fully matched rank higher.
  score += Math.max(0, 12 - (t.length - q.length));
  return { score, indices };
}
