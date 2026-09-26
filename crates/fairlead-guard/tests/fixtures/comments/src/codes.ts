export const a = 1

// Retries twice, since the first call can race the cache (ABC-12).
export const b = 2

// Two pointers are fine too (ABC-12, ABC-13). More words after.
export const c = 3

// See ABC-12 for the story, and ABC-12 again.
export const d = 4

/** One line with its pointer (ABC-12) */
export const e = 5

// A list that isn't a pointer (ABC-12 and more)
export const f = 6
