export const a = 1 // This used to be two.

export const b = "// not a comment ABC-12"

export function C() {
  return (
    <div>
      // plain text, not a comment
      {/* Ada wrote this */}
    </div>
  )
}

export const d = 1 /* a */ // You must not.
