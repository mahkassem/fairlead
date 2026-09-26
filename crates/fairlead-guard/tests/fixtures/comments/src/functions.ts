export function long() {
  const a = 1
  const b = 2
  const c = 3
  const d = 4
  return a + b + c + d
}

class K {
  @logged()
  method() {
    const inner = () => {
      const a = 1
      const b = 2
      const c = 3
      const d = 4
      return a + b + c + d
    }
    const x = 1
    const y = 2
    const z = 3
    return inner
  }
}

function logged() {
  return () => {}
}

declare const sql: (s: TemplateStringsArray, ...v: unknown[]) => string

export function outer() {
  const q = (id: number) =>
    sql<{ n: number; m: string }>`
      select ${id}
      from t
    `
  return q
}
