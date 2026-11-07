# Install recipes → koma-safety bake

Taxonomy (2345 names) ≠ installable. Each Linux CLI needs a recipe, then a pack bake.

```
categorizer / manual
  → recipes.jsonl (this dir)
  → scripts/gen_pack_from_recipes.py
  → catalog/packs/{smoke,standard,extended}.json
  → image build TARGET=minimal|standard|extended
```

`skip` method = catalog-only (`ready: false`). Never apt-as-root at guest runtime (RO root).
