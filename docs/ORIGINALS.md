# Original content from your keyboard's vendor software

Halcyon ships no vendor files: no product photos, animations or databases. They belong to the manufacturer.
If you own the vendor software, Halcyon can read a few files from **your own copy**, kept in a private folder
on your computer (`~/.halcyon/vendor/<keyboard id>/`).

| What you get | AMG65 file in the vendor app folder |
|---|---|
| Product photo on the Keyboards page and in the switcher | `skins/theme1/keyboard/home_keyboard.png` |
| **Restore original** boot animation, and the factory matrix styles | `db/ledlayer_v2.data` |
| **Restore original screen** | `gif/LEOBOG AMG65/0.gif` |

Everything else in Halcyon works without these.

## Importing

If the vendor software is installed, point the import tool at its folder (the one containing `DeviceDriver.exe`):

```
halcyon import-originals leobog-amg65 "C:/Program Files (x86)/LEOBOG AMG65"
```

If you only have the installer, unpack it without running it using [innoextract](https://constexpr.org/innoextract/):

```
innoextract -e -d unpacked "LEOBOG AMG65 Driver.exe"
halcyon import-originals leobog-amg65 unpacked/app
```

You can also use your own photo instead: **Keyboards → hover the picture → Change photo**.

## For definition authors

A definition's `originals` block lists, per item, `path` (where the file sits inside the vendor app folder) and
`file` (the name Halcyon stores it under). Never add the files themselves to the repository.
