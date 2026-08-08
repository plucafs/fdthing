# fd-gui — TODO

## Da fare
- [ ] Ricerca full-text nel contenuto dei file
- [ ] Salvataggio/caricamento di "preset" di ricerca
- [ ] Ricerca live (debounce) mentre si digita il pattern

## Fatto
- [x] Icona SVG personalizzata (lente + documento, tema viola) — assets/icon.svg → assets/icon_128.png
- [x] Input numerico per cambiare lo scaling della UI (DragValue + `ctx.set_zoom_factor()`, persistito)
- [x] Aprire file al click (click sinistro: apri file, click destro: apri cartella contenente)
- [x] Filtro per estensione file (input testuale, es. `rs, py, txt`)
- [x] Ordinamento risultati (nome/percorso, crescente/decrescente)
- [x] Persistenza lista directory (e opzioni) tra le sessioni — `~/.config/fd-gui/config.json`
- [x] Scheletro app egui/eframe
- [x] Lista directory con toggle on/off, aggiungi/rimuovi, All/None
- [x] Ricerca in background thread (non blocca la UI) con Stop
- [x] Opzioni: case sensitive, regex, hidden, symlinks, .gitignore
