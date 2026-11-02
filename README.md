# CreateCalculated

Калькулятор производства для мода Create (Minecraft 1.21.1, Create 6): строит граф производства, как Factorio Calculator. Вы задаёте предмет и количество в минуту, а получаете машины, потоки, сырьё, остатки и нагрузку в SU.

| Папка | Что внутри |
|---|---|
| `core` | Ядро на Rust: формулы машин, импорт рецептов, решатель |
| `server` | HTTP API над ядром (axum) |
| `mockups` | `graph.html`: прототип интерфейса, граф на настоящем API |
| `docs` | Документация |
| `tools` | Датапак для замеров в игре, см. [docs/measurements.md](docs/measurements.md) |

## Запуск

Нужен [Rust](https://rustup.rs). Рецепты в репозитории не хранятся, их один раз собирают из своих JAR, см. [docs/import.md](docs/import.md).

```sh
cargo run --release --bin createcalc-import -- -o data/bundle.json minecraft.jar neoforge-universal.jar create.jar
cargo run --release --bin createcalc-server
cargo test --workspace
```

Документация: [как работает расчёт](docs/architecture.md), [формулы машин](docs/formulas.md), [API](docs/api.md).

## Ветки

`Developer` для текущей работы, `main` только для крупных релизов.

## Данные игры

На ассеты Create (`assets/`) действует лицензия All Rights Reserved, а данные Minecraft принадлежат Mojang. Поэтому текстуры, переводы и наборы рецептов в репозиторий не входят, импорт идёт локально из файлов пользователя.
