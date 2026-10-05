/**
 * How a session is named in a list: the work it ran against, and enough of its
 * identity to tell two rows apart. Shared by Sessions and by the Dashboard's
 * lane table, so the same session reads the same way on both pages.
 *
 * Nothing here invents a fact. A missing repository or branch is said to be
 * unknown; no transcript title exists, so the identifier stays visible.
 */

/** Enough of the canonical ID to tell two rows apart without filling the cell. */
export const shortId = (id: string) => id.replace(/^(codex|cursor)-/, '').slice(0, 8);

/** A reviewer label stays separate from saved and native titles. */
export const displayTitle = (title: string | null, automatedReview: boolean) =>
  title ?? (automatedReview ? 'Automated review' : null);

/** The repository's own name, out of a stored path, or nothing when absent. */
export const repoName = (repo: string | null | undefined) =>
  repo?.replaceAll('\\', '/').split('/').filter(Boolean).at(-1) ?? null;

/**
 * The visible lead of a session row. A branch alone would read as a
 * repository, so an unknown repository is named before it; an unknown branch
 * is left to the full title rather than spending the row's width on saying so
 * twice.
 */
export const contextLead = (repo: string | null, branch: string | null) =>
  `${repoName(repo) ?? 'Unknown repository'}${branch ? ` · ${branch}` : ''}`;

/** The whole stored context, for a title and for assistive technology. */
export const contextTitle = (repo: string | null, branch: string | null) =>
  `${repo ?? 'Unknown repository'} · ${branch ?? 'Unknown branch'}`;
