# Install recipes → koma-safety bake

Taxonomy (2345 names) ≠ installable. Each Linux CLI needs a recipe, then a pack bake.

```
categorizer / manual
  → recipes + install_recipes/buckets/*.bins.txt
  → catalog/packs/{smoke,minimal,<domain>}.json  (no standard/extended)
  → image build KOMA_COMPOSITION=minimal|recon|web|…
```

`skip` method = catalog-only (`ready: false`). Never apt-as-root at guest runtime (RO root).
