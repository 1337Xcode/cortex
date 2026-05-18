import re

p = r"src/components/landing/BentoInteractives.tsx"
with open(p, encoding="utf-8") as f:
    c = f.read()

new_fn = """export function McpToolGrid() {
  return (
    <ul className="mt-2 space-y-1 text-[11px] text-muted-foreground leading-snug">
      {mcpTools.map((t) => (
        <li key={t.name} className="flex justify-between gap-2 tabular-nums">
          <span className="truncate">{t.name}</span>
          <span className="font-semibold text-foreground shrink-0">{t.count}</span>
        </li>
      ))}
      <li className="pt-1 border-t border-border/60 text-[10px]">
        <span className="font-medium text-foreground">ask</span> router · 5 in smart mode
      </li>
    </ul>
  );
}"""

c2, n = re.subn(
    r"export function McpToolGrid\(\) \{[\s\S]*?\n\}\n\nexport function IndexingMeter",
    new_fn + "\n\nexport function IndexingMeter",
    c,
    count=1,
)
if n != 1:
    raise SystemExit(f"replace failed: {n}")
with open(p, "w", encoding="utf-8") as f:
    f.write(c2)
print("patched")
