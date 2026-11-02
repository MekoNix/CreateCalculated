# HTTP API

Запуск: `createcalc-server [bundle.json] [адрес] [папка картинок]`, по умолчанию `data/bundle.json`, `127.0.0.1:3000` и `data/icons`. Страница открывается по адресу сервера, `GET /`. Для доступа из локальной сети запускайте с адресом `0.0.0.0:3000`. Авторизации нет, поэтому не открывайте сервер в интернет. CORS открыт, чтобы фронтенд можно было открывать отдельно. Ошибки приходят как `{"error": "..."}`.

| Метод и путь | Что делает |
|---|---|
| `GET /api/health` | Проверка работы |
| `GET /api/items?q=латун&lang=ru&limit=50` | Поиск предметов и жидкостей |
| `GET /api/recipes/{id}` | Один рецепт, например `create:mixing/brass_ingot` |
| `GET /api/producers?item=minecraft:iron_ingot` | Рецепты, которые дают предмет, сначала те, где он основной выход |
| `GET /api/consumers?item=minecraft:flint` | Рецепты, которые используют предмет, напрямую или через тег |
| `GET /api/config/default` | Значения Create по умолчанию |
| `POST /api/config/parse` | Текст `create-server.toml` в теле, в ответ конфиг |
| `POST /api/plan` | Расчёт производства |
| `POST /api/power` | Сколько источников нужно на заданные SU |
| `GET /api/icons/index.json` | Какие картинки предметов есть. 404, если они не собраны, см. [картинки предметов](icons.md) |
| `GET /api/icons/{путь}` | Картинка PNG по пути из индекса. 404, если её нет |

## POST /api/plan

Обязательно только `targets`. `recipe_choice` делает рецепт единственным источником предмета: другие рецепты, дающие его даже побочно, отключаются, если сами не выбраны.

```json
{
  "targets": [{ "item": "create:precision_mechanism", "per_minute": 60 }],
  "rpm": 256,
  "batch": 1,
  "stock": ["minecraft:iron_ingot"],
  "recipe_choice": { "minecraft:iron_ingot": "minecraft:iron_ingot_from_blasting_raw_iron" },
  "disabled_recipes": [],
  "overrides": { "create:mixing/brass_ingot": { "rpm": 64, "machine": "mixer", "batch": 1 } },
  "include_heat": true,
  "base_resources": null,
  "mode": "normal",
  "belt_rpm": 32,
  "stack": 1,
  "stack_sizes": { "minecraft:cobblestone": 64 },
  "stop_at_materials": false,
  "branches": [{ "recipe": "minecraft:arrow", "input": "minecraft:flint", "per_minute": null }],
  "config": {}
}
```

В ответе:

- `nodes`: поле `type` равно `recipe`, `gather` (добыча), `stock` (склад), `generate` (бесконечный источник в режиме `full_auto`), `surplus` (остаток) или `target` (цель). У `recipe` есть `produces` (какой выход нужен плану: цель, иначе самый используемый), `loads` (машины, RPM, число, SU, для вентилятора длина потока), `alternatives` и `machine_options`.
- `edges`: `from`, `to`, `res`, `per_minute`, для предметов `transport`: `stack`, `belt_rpm`, `belts`, `min_belt_rpm`, `funnels`, `needs_buffer`.
- `total_su`, `machines`, `heat` (нет при `include_heat: false`), `warnings`, `branches` (во что вылился каждый отросток), `manual` (в режиме `full_auto`: что всё ещё добывать вручную), `build` (что построить: машины, чаши, бёрнеры, воронки, линии лент и ящики для буферов, `item` и `count`). Валы, трубы, насосы и источники энергии в `build` не входят. Материалы на постройку считаются отдельным запросом с этими предметами в `targets` и `stop_at_materials: true`: расчёт останавливается на готовых материалах из `core/data/material_resources.txt` (слитки, сплав, доски) и не идёт до руды.

Отросток `branches` добавляет рецепт от предмета схемы и фиксирует этот рецепт для его основного выхода. Без `per_minute` он забирает весь остаток `input` и следует за ним при изменении схемы. С `per_minute` количество постоянное, а если остатка не хватает, решатель доберёт разницу.

Если `rpm` выходит за `max_rotation_speed` из конфига или список целей пуст, вернётся 400.

## POST /api/power

```json
{ "su": 40960, "sails": 64, "config": {} }
```

В ответе `boilers` (уровни, двигатели, баки, вода, бёрнеры) и `sources` (сколько каждого источника покроет нагрузку).
