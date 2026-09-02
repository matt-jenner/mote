# Fredoka in Mote

| Field | Value |
| --- | --- |
| Family | Fredoka |
| Source | [Google Fonts repository](https://github.com/google/fonts/tree/main/ofl/fredoka) |
| Licence | SIL Open Font License 1.1 |
| Files | `Fredoka-Variable.ttf`, `Fredoka-Variable.woff2`, `OFL.txt` |
| Wordmark setting | Weight 400, width 96, tracking -0.035em |
| Available axes | Weight 300-700, width 75-125 |

The binaries retain the upstream naming and variation tables. The source font reports the internal base name `Fredoka Light`; the Regular named instance uses weight 400. The CSS family label below deliberately exposes it to the application as `Fredoka` without modifying the font binary.

```css
@font-face {
  font-family: "Fredoka";
  src: url("./Fredoka-Variable.woff2") format("woff2-variations");
  font-style: normal;
  font-weight: 300 700;
  font-stretch: 75% 125%;
  font-display: swap;
}

.mote-wordmark {
  font-family: "Fredoka", sans-serif;
  font-variation-settings: "wght" 400, "wdth" 96;
  letter-spacing: -0.035em;
}
```

## Checksums

```text
2ba02e68b152868aef9ba28e24b3648c7d457fe6f25c761f2c2c53fb61a73fc8  Fredoka-Variable.ttf
23a171c4d595d134261436637f2c7e7e614264577a54779dbaccfd228eb289ad  Fredoka-Variable.woff2
5c9e7eee5c6b25f4b05b8d53b2e470ea4962f9ced742d044a98f7d95d1375bab  OFL.txt
```

Redistribute `OFL.txt` with either font file. The licence permits use, modification, and redistribution subject to its conditions.
