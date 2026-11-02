execute at @e[type=marker,tag=ccalc,limit=1] run function ccalc:count
scoreboard players operation #delta ccalc = #n ccalc
scoreboard players operation #delta ccalc -= #start ccalc
# Items per minute.
scoreboard players operation #perMin ccalc = #delta ccalc
scoreboard players operation #perMin ccalc *= #1200 ccalc
scoreboard players operation #perMin ccalc /= #ticks ccalc
# Ticks per 100 items, which is the cycle time multiplied by 100.
scoreboard players operation #per100 ccalc = #ticks ccalc
scoreboard players operation #per100 ccalc *= #100 ccalc
execute if score #delta ccalc matches 1.. run scoreboard players operation #per100 ccalc /= #delta ccalc
tellraw @a [{"text":"Замер: ","color":"green"},{"score":{"name":"#delta","objective":"ccalc"},"color":"white"},{"text":" шт за "},{"score":{"name":"#ticks","objective":"ccalc"},"color":"white"},{"text":" тиков = "},{"score":{"name":"#perMin","objective":"ccalc"},"color":"aqua"},{"text":" в минуту"}]
execute if score #delta ccalc matches 1.. run tellraw @a [{"text":"Тиков на 100 предметов: ","color":"green"},{"score":{"name":"#per100","objective":"ccalc"},"color":"aqua"},{"text":" (цикл = это число / 100)"}]
execute if score #delta ccalc matches ..0 run tellraw @a {"text":"Предметов не прибавилось: проверьте линию.","color":"red"}
execute if score #n ccalc matches 1600.. run tellraw @a {"text":"Ящик почти полон (одинарный вмещает 1728), результат может быть занижен. Очистите его и повторите.","color":"red"}
