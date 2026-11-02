execute store result score #c ccalc run data get storage ccalc:m items[0].count
scoreboard players operation #n ccalc += #c ccalc
data remove storage ccalc:m items[0]
execute if data storage ccalc:m items[0] run function ccalc:count_loop
