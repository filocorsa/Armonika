# Armonika - brand

## Colori

| Nome | Hex | Uso |
| --- | --- | --- |
| Disco | `#161616` | Vinile e favicon su sfondo chiaro |
| Testo scuro | `#111111` | Scritta "armonika" su sfondo chiaro |
| Ruggine | `#C2452D` | Ingranaggio, unico colore d'accento |
| Crema | `#F4F1EC` | Scritta su sfondo scuro |
| Disco chiaro | `#E8E6E1` | Vinile su sfondo scuro |
| Sfondo scuro | `#0F1115` | Sfondo delle versioni dark e dell'app |

## Font

Scritta: **Inter Display Black** (licenza SIL Open Font). Nei file SVG il testo è già convertito in tracciati, quindi non serve installare il font.

## File

```
armonika-logo-light.svg / .png / .jpg   logo esteso, sfondo chiaro
armonika-logo-dark.svg  / .png / .jpg   logo esteso, sfondo scuro
armonika-logo-banner.svg                logo esteso dark con il suo sfondo scuro arrotondato, leggibile su sfondi chiari e scuri (README)
armonika-icon-light.svg / .png / .jpg   icona, sfondo chiaro
armonika-icon-dark.svg  / .png / .jpg   icona, sfondo scuro
armonika-favicon.svg                    favicon semplificata (8 denti, niente solchi)

mono/    versioni a un colore (nero e bianco) per badge, stampa e sfondi colorati
web/     favicon.ico, favicon.svg, apple-touch-icon, icone PWA, site.webmanifest
github/  avatar (500x500) e social preview (1280x640, chiara e scura)
```

## Favicon nell'HTML

```html
<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">
```

## Regole d'uso

- L'arancione si usa solo per l'ingranaggio: non colorare la scritta.
- Lascia intorno al logo almeno lo spazio di metà vinile.
- Su sfondi chiari usa le versioni light, su quelli scuri le dark, su sfondi colorati o foto le mono.
- Non deformare, ruotare o ricolorare il vinile.
