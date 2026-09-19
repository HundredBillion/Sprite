# Adwaita Sans static UI faces

Sprite embeds regular (weight 400) and bold (weight 700) Adwaita Sans faces
for Linux UI text. They are static derivatives of the Adwaita Sans 50.0
variable font, instantiated at optical size 14. The font is licensed under
the SIL Open Font License 1.1; see [OFL-1.1.txt](OFL-1.1.txt).

Source: GNOME Adwaita Fonts 50.0,
https://download.gnome.org/sources/adwaita-fonts/50/adwaita-fonts-50.0.tar.xz

- Source archive SHA-256: `4c927fbfeec1c503801ba510c2c94e0054c82c522cf7ba0d3be5d4d41fcf5c86`
- Source `sans/AdwaitaSans-Regular.ttf` SHA-256: `8381c33b9a44f066f2b99dba3d416a2342891e28c956a35dfd8d16ee2987e6d4`
- Original `LICENSE` SHA-256: `459687971d21c53923c1d1c9c062ec273a7ea03226b36195b79ec6af7d98dc81`
- Static regular SHA-256: `e2e3e6aede5c4245c7c3260e9d2ef4437a2ab0dc7a446ee14127ad03db55dff3`
- Static bold SHA-256: `fd309de15fe71c3b9cd68ee59266ede17a8af00a25ac39852b4b0ad7bad61080`

To reproduce, download the pinned source archive and run:

```sh
uv run --with fonttools==4.65.0 python scripts/generate-adwaita-sans-static.py adwaita-fonts-50.0.tar.xz
```

The script checks archive, source, license, and output hashes. FontTools is a
maintainer tool; Sprite requires neither Python nor FontTools at runtime.
