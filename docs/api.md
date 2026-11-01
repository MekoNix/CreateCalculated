# HTTP API

Запуск: `createcalc-server [bundle.json] [адрес]`, по умолчанию `data/bundle.json` и `127.0.0.1:3000`. CORS открыт, чтобы фронтенд можно было открывать отдельно. Ошибки приходят как `{"error": "..."}`.

| Метод и путь | Что делает |
|---|---|
| `GET /api/health` | Проверка работы |
| `GET /api/items?q=латун&lang=ru&limit=50` | Поиск предметов и жидкостей |
| `GET /api/recipes/{id}` | Один рецепт, например `create:mixing/brass_ingot` |
| `GET /api/config/default` | Значения Create по умолчанию |
| `POST /api/config/parse` | Текст `create-server.toml` в теле, в ответ конфиг |
| `POST /api/plan` | Расчёт производства |
| `POST /api/power` | Сколько источников нужно на заданные SU |

## POST /api/plan

Обязательно только `targets`.

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
  "config": {}
}
```

В ответе:

- `nodes`: поле `type` равно `recipe`, `gather` (добыча), `stock` (склад), `generate` (бесконечный источник в режиме `full_auto`), `surplus` (остаток) или `target` (цель). У `recipe` есть `loads` (машины, RPM, число, SU, для вентилятора длина потока), `alternatives` и `machine_options`.
- `edges`: `from`, `to`, `res`, `per_minute`.
- `total_su`, `machines`, `heat` (нет при `include_heat: false`), `warnings`, `manual` (в режиме `full_auto`: что всё ещё добывать вручную).

Если `rpm` выходит за `max_rotation_speed` из конфига или список целей пуст, вернётся 400.

## POST /api/power

```json
{ "su": 40960, "sails": 64, "config": {} }
```

В ответе `boilers` (уровни, двигатели, баки, вода, бёрнеры) и `sources` (сколько каждого источника покроет нагрузку).
