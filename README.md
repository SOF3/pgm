# Propagative Gravitation Model (PGM)

PGM is an probabilistic model to simulate gravitation pull (i.e. "falling")
in a block-based (2D/3D) world.
This works by emitting "signals" at random grids
and randomly propagating these signals to adjacent blocks
that can "stick" them together to prevent falling.
The signals can be absorbed by "sink tiles" (blocks that never fall)
which consume the signal without propagating them out.
If a connected group of blocks have no sink tiles,
the signals generated in this group will accumulate indefinitely.
When the signal strength is sufficiently high,
we can determine that these blocks are "floating" and should start falling down.

## Is this working now?

This repository contains some basic simulation code for a 2D world
and renders the signal strength on a 2D terminal UI.

Currently the algorithm works very poorly because the parameters have not been tuned.
Some more rigorous calculations need to be done to select the right parameters.
