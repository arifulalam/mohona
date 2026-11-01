# Translating Mohona into your language

The Mohona window speaks English, বাংলা, हिन्दी, اردو, العربية, Español, Français, Deutsch, Português, Русский, 中文, 日本語, Bahasa Indonesia and Türkçe. The person picks the language in the box at the top right (the first start follows the browser's or Windows' language). The choice is remembered.

Anyone can add another language, or improve one, without writing code. The translations that came with Mohona were written without a native speaker's review: if you speak one of these languages and see a word that sounds wrong, please fix it and send the file back.

## What is translated

The app window: every button, label, message and table. Not translated: messages that the engine itself writes (for example "the address must start with http://"), messages written by the extension's background (such as "Sent to Mohona: ...") and the names of your own categories and queues. The browser extension's popup is translated too, with its own, smaller set of files (see below). 

## Add a new language (about an hour for the whole window)

You need the `app` folder and Node.js (the same as for building the app).

1. In `app`, run `npm run i18n -- --new ne` (use the language's code instead of `ne`: `ne` Nepali, `ta` Tamil, `pa` Punjabi, `ko` Korean, `it` Italian, `fa` Persian, `pt-BR` Brazilian Portuguese, ...). It creates `src/locales/ne.json`. Without Node you can copy `src/locales/_template.json` to `src/locales/ne.json` yourself.
2. Open the file in any text editor (save it as **UTF-8**). It looks like this:

   ```json
   {
     "_name": "Your language's name, as its speakers write it",
     "_english": "The same name in English",
     "_dir": "ltr",
     "Settings": "",
     "Pause {name}": "",
     "{done} of {total}": ""
   }
   ```

   - `_name`: the language's own name, as it should appear in the list ("नेपाली").
   - `_english`: the same name in English ("Nepali"). The list shows both, "नेपाली · Nepali", so people who cannot read the script still know which language it is.
   - `_dir`: `"ltr"` for left to right, `"rtl"` for right to left (Arabic, Urdu, Persian, Hebrew).
   - On the left of each line is the English text. **Never change it.** Write the translation between the second pair of quotes.
   - `{name}`, `{count}`, `{done}` and the like are filled in by the program. Keep them exactly as they are, and move them where they belong in your sentence: `"{done} of {total}"` may become `"{total} मध्ये {done}"`.
   - A text left empty (`""`) is shown in English, so you can translate in several sittings and the window still works.
   - Keep short texts short: they are on buttons.
   - A `"` inside a text is written `\"`.
3. Check your work: `npm run i18n:check` lists every language with how many texts are translated, and says what is missing or wrong (a `{name}` that was changed, a text that no longer exists).
4. Start the app (`npm run dev`, or `npm run tauri dev`). Your language is in the list already, with both names ("नेपाली · Nepali"). Click through every window (Add, Settings, Queues, Schedule, Categories, Details) and look for texts that are cut off or sound strange.
5. Send us the file `ne.json` (a pull request, an e-mail, or an issue with the file attached) so everyone gets it. Nothing else has to change: the program finds every file in `src/locales` by itself.

## The browser extension's popup

The popup of the extension has its own language files in `extension/src/locales` (about 55 short texts, most of them the same as in the app) and the same format. In the `extension` folder, `npm run i18n -- --new ne` starts `src/locales/ne.json`; fill it in as above, run `npm run build`, and load the extension again. The popup has its own language box and uses the browser's language the first time.

## Improve an existing language

Edit its file in `src/locales` (for example `bn.json`) and run `npm run i18n:check`. When a new version of Mohona adds texts, run `npm run i18n:check` and translate what it reports as missing: the English text shows meanwhile.

## For developers

In the code, every text is written `t("English text")` (or `t("Pause {name}", { name })`), and texts in tables are marked `tr("English text")` and translated when shown. `npm run i18n` writes `src/locales/_template.json` with every text found in the code; the tests fail when a language lacks a text, keeps a text that no longer exists, or changes a `{name}`.
