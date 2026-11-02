# Usage: /function ccalc:start {ticks:1200}
$scoreboard players set #ticks ccalc $(ticks)
execute unless entity @e[type=marker,tag=ccalc] run tellraw @a {"text":"Сначала отметьте ящик: /function ccalc:mark","color":"red"}
execute at @e[type=marker,tag=ccalc,limit=1] run function ccalc:count
scoreboard players operation #start ccalc = #n ccalc
$schedule function ccalc:finish $(ticks)t
$tellraw @a {"text":"Замер пошёл: $(ticks) тиков","color":"yellow"}
