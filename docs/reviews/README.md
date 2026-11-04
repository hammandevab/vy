# Granskningssidor

En sida per PR, skriven av en agent som **inte** skrev koden. Människan läser den här sidan,
aldrig koden. Filnamn: `<PR-nummer>-<slug>.md`. Max en skärm.

```markdown
# PR #12 – vy-dl: klippstack

**Vad ändrades.** Två meningar.

**Spec-id som berörs.** SPEC-6.1.4, SPEC-C.1.1.

**Risk.** Vad kan gå fel om det här är fel, och vem drabbas.

**Bevis.** Vilka tester går från ignorerade till gröna. Vilka guldbilder ändrades och varför.

**Nya beroenden.** Inga. / `foo 1.2` – motivering.

**Hotbild.** Oförändrad. / Ny ABI-import: länk till hotanalysen.

**Rekommendation.** Merge / Merge efter X / Avslå.
```
