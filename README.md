# CreateCalculated

Калькулятор производства для мода Create (Minecraft 1.21.1, Create 6): строит граф производства, как Factorio Calculator. Вы задаёте предмет и количество в минуту, а получаете машины, потоки, сырьё, остатки и нагрузку в SU.

| Папка | Что внутри |
|---|---|
| `core` | Ядро на Rust: формулы машин, импорт рецептов, решатель |
| `server` | HTTP API над ядром (axum) |
| `icons` | `createcalc-icons`: собирает картинки предметов из JAR на вашем компьютере, см. [docs/icons.md](docs/icons.md) |
| `mockups` | `graph.html`: прототип интерфейса, граф на настоящем API |
| `docs` | Документация |
| `tools` | Датапак для замеров в игре, см. [docs/measurements.md](docs/measurements.md) |

## Запуск

Нужен [Rust](https://rustup.rs) и C-компилятор: он нужен инструменту картинок для HTTPS. Один сервер собирается и без него: `cargo build -p createcalc-server`. Рецепты в репозитории не хранятся, их один раз собирают из своих JAR, см. [docs/import.md](docs/import.md).

```sh
cargo run --release --bin createcalc-import -- -o data/bundle.json minecraft.jar neoforge-universal.jar create.jar
cargo run --release --bin createcalc-server
cargo test --workspace
```

Картинки предметов необязательны, без них страница показывает буквы. Команда сама скачает игру и Create в `data/`, см. [docs/icons.md](docs/icons.md):

```sh
cargo run --release --bin createcalc-icons
```

Документация: [как работает расчёт](docs/architecture.md), [формулы машин](docs/formulas.md), [API](docs/api.md), [картинки предметов](docs/icons.md).

## Ветки

`Developer` для текущей работы, `main` только для крупных релизов.

## Данные игры

На ассеты Create (`assets/`) действует лицензия All Rights Reserved, а данные Minecraft принадлежат Mojang. Поэтому текстуры, картинки предметов, переводы и наборы рецептов в репозиторий не входят, импорт и сборка картинок идут локально из файлов пользователя.
