# Sums the items of the marked container into #n. Runs at the marker.
data remove storage ccalc:m items
data modify storage ccalc:m items set from block ~ ~ ~ Items
scoreboard players set #n ccalc 0
execute if data storage ccalc:m items[0] run function ccalc:count_loop
