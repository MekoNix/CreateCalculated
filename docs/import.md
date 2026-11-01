# Импорт рецептов

Импорт собирает `data/bundle.json` из JAR на вашем компьютере. Эта папка не попадает в git.

```sh
cargo run --release --bin createcalc-import -- -o data/bundle.json <minecraft.jar> <neoforge-universal.jar> <create.jar>
```

Порядок важен: сначала игра, потом NeoForge, потом моды. Поздние файлы заменяют рецепты с тем же id и дополняют теги.

| Файл | Где искать (Modrinth App) |
|---|---|
| Minecraft | `meta/versions/1.21.1-<neoforge>/1.21.1-<neoforge>.jar` |
| NeoForge | `meta/libraries/net/neoforged/neoforge/<версия>/neoforge-<версия>-universal.jar` |
| Create | `profiles/<профиль>/mods/create-1.21.1-6.0.10.jar` |

Команда выводит статистику: сколько рецептов загружено, сколько отброшено по условиям (например, нужен другой мод), какие типы пропущены и что не разобралось. На Create 6.0.10 получается около 2600 рецептов без ошибок разбора.

Пропускаются спецрецепты без постоянного результата (покраска брони, копирование книг) и готовка на костре.
