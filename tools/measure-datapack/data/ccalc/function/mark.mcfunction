# Run while standing on the container or inside its block column one above.
kill @e[type=marker,tag=ccalc]
scoreboard players set #found ccalc 0
execute align xyz positioned ~0.5 ~0.5 ~0.5 if block ~ ~ ~ #ccalc:container run function ccalc:mark_here
execute if score #found ccalc matches 0 align xyz positioned ~0.5 ~-0.5 ~0.5 if block ~ ~ ~ #ccalc:container run function ccalc:mark_here
execute if score #found ccalc matches 0 run tellraw @s {"text":"Ящик или бочка не найдены под ногами. Встаньте прямо на него.","color":"red"}
