export const a = 1

// Retries twice, since the first call can race the cache (T1024).
export const b = 2

// Two pointers are fine too (T1024, T1025). More words after.
export const c = 3

// See T1024 for the story, and T1024 again.
export const d = 4

/** One line with its pointer (T1024) */
export const e = 5

// A list that isn't a pointer (T1024 and more)
export const f = 6
