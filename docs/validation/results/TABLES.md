## Ordinary primary

21 measured repetitions per cell, two warmups. Times in milliseconds; ratios current/older, below 1 favors current. Cold is the additive preparation-charge model described in REPORT.md.

| Regime | n | Control | Current reused ms | Older reused ms | Paired ratio [bootstrap 95%] | Current cold ms | Older cold ms |
|---|---:|---|---:|---:|---:|---:|---:|
| alias | 8 | old-active | 0.370 | 0.035 | 10.572 [10.298, 11.776] | 0.390 | 0.050 |
| alias | 32 | old-active | 1.434 | 0.131 | 11.101 [10.636, 11.415] | 1.482 | 0.170 |
| alias | 128 | old-active | 7.090 | 0.536 | 13.288 [13.037, 13.700] | 7.237 | 0.668 |
| degree | 8 | old-active | 0.469 | 0.048 | 9.725 [9.633, 10.859] | 0.483 | 0.059 |
| degree | 32 | old-active | 1.985 | 0.198 | 10.077 [9.457, 10.303] | 2.029 | 0.235 |
| degree | 128 | old-active | 14.779 | 0.816 | 18.056 [11.577, 19.813] | 14.926 | 0.944 |
| sparse | 8 | old-active | 0.162 | 0.021 | 7.765 [7.435, 8.646] | 0.173 | 0.028 |
| sparse | 32 | old-active | 0.589 | 0.080 | 7.349 [7.152, 8.042] | 0.626 | 0.109 |
| sparse | 128 | old-active | 2.519 | 0.310 | 8.868 [8.093, 12.057] | 2.717 | 0.435 |
| dense | 8 | old-active | 4.650 | 0.196 | 23.108 [9.489, 23.533] | 4.669 | 0.210 |
| dense | 32 | old-active | 70.743 | 10.946 | 6.568 [6.346, 8.460] | 70.808 | 10.997 |
| dense | 128 | old-active | 1639.712 | 531.770 | 3.104 [3.048, 3.131] | 1639.956 | 531.968 |
| cyclic | 8 | old-active | 1.749 | 0.611 | 2.982 [2.840, 3.053] | 1.779 | 0.637 |
| cyclic | 32 | old-active | 13.907 | 14.100 | 0.991 [0.823, 0.995] | 14.020 | 14.198 |
| cyclic | 128 | old-active | 40.308 | 124.586 | 0.323 [0.317, 0.326] | 40.701 | 124.959 |

## Ordinary secondary

7 measured repetitions per cell, two warmups. Times in milliseconds; ratios current/older, below 1 favors current. Cold is the additive preparation-charge model described in REPORT.md.

| Regime | n | Control | Current reused ms | Older reused ms | Paired ratio [bootstrap 95%] | Current cold ms | Older cold ms |
|---|---:|---|---:|---:|---:|---:|---:|
| alias | 8 | old-active | 0.378 | 0.031 | 12.327 [12.117, 13.833] | 0.399 | 0.048 |
| alias | 8 | old-global | 0.378 | 0.036 | 10.398 [10.188, 12.353] | 0.399 | 0.053 |
| alias | 32 | old-active | 1.451 | 0.121 | 12.401 [11.945, 13.093] | 1.510 | 0.165 |
| alias | 32 | old-global | 1.451 | 0.140 | 10.289 [9.352, 11.036] | 1.510 | 0.193 |
| alias | 128 | old-active | 7.117 | 0.471 | 15.144 [13.478, 15.583] | 7.284 | 0.606 |
| alias | 128 | old-global | 7.117 | 0.621 | 11.429 [11.111, 12.643] | 7.284 | 0.750 |
| degree | 8 | old-active | 0.480 | 0.044 | 10.716 [9.922, 11.862] | 0.494 | 0.055 |
| degree | 8 | old-global | 0.480 | 0.040 | 12.286 [11.579, 12.890] | 0.494 | 0.051 |
| degree | 32 | old-active | 1.916 | 0.183 | 10.389 [9.363, 10.535] | 1.961 | 0.224 |
| degree | 32 | old-global | 1.916 | 0.227 | 8.582 [8.092, 8.816] | 1.961 | 0.264 |
| degree | 128 | old-active | 8.641 | 0.763 | 11.225 [10.887, 11.903] | 8.799 | 0.893 |
| degree | 128 | old-global | 8.641 | 2.239 | 3.887 [3.574, 4.105] | 8.799 | 2.369 |
| sparse | 8 | old-active | 0.158 | 0.017 | 9.492 [8.349, 10.380] | 0.169 | 0.025 |
| sparse | 8 | old-global | 0.158 | 0.014 | 11.207 [11.050, 14.372] | 0.169 | 0.022 |
| sparse | 32 | old-active | 0.578 | 0.073 | 7.943 [7.567, 8.020] | 0.616 | 0.105 |
| sparse | 32 | old-global | 0.578 | 0.061 | 9.861 [9.343, 10.545] | 0.616 | 0.089 |
| sparse | 128 | old-active | 2.425 | 0.273 | 8.692 [8.439, 8.978] | 2.558 | 0.381 |
| sparse | 128 | old-global | 2.425 | 0.207 | 11.797 [10.696, 12.203] | 2.558 | 0.311 |
| dense | 8 | old-active | 1.649 | 0.174 | 9.256 [9.147, 9.800] | 1.670 | 0.189 |
| dense | 8 | old-global | 1.649 | 0.419 | 3.859 [3.820, 3.974] | 1.670 | 0.433 |
| dense | 32 | old-active | 33.034 | 4.920 | 6.700 [6.571, 6.827] | 33.102 | 4.973 |
| dense | 32 | old-global | 33.034 | 96.432 | 0.342 [0.337, 0.346] | 33.102 | 96.485 |
| dense | 128 | old-active | 773.369 | 252.513 | 3.088 [3.016, 3.166] | 773.609 | 252.707 |
| dense | 128 | old-global | 773.369 | 28535.530 | 0.028 [0.027, 0.028] | 773.609 | 28535.718 |
| cyclic | 8 | old-active | 1.650 | 0.577 | 2.895 [2.839, 2.998] | 1.681 | 0.604 |
| cyclic | 8 | old-global | 1.650 | 1.749 | 0.949 [0.922, 0.983] | 1.681 | 1.775 |
| cyclic | 32 | old-active | 7.563 | 7.856 | 0.967 [0.920, 0.987] | 7.674 | 7.952 |
| cyclic | 32 | old-global | 7.563 | 101.945 | 0.075 [0.072, 0.075] | 7.674 | 102.042 |
| cyclic | 128 | old-active | 42.609 | 128.113 | 0.327 [0.318, 0.374] | 43.063 | 128.482 |
| cyclic | 128 | old-global | 42.609 | 6908.759 | 0.006 [0.006, 0.006] | 43.063 | 6909.128 |

## Search default

21 measured repetitions per cell, two warmups. Times in milliseconds; ratios current/older, below 1 favors current. Cold is the additive preparation-charge model described in REPORT.md.

| Regime | n | Control | Current reused ms | Older reused ms | Paired ratio [bootstrap 95%] | Current cold ms | Older cold ms |
|---|---:|---|---:|---:|---:|---:|---:|
| common | 8 | old-active | 0.478 | 0.081 | 5.890 [5.718, 5.988] | 0.493 | 0.100 |
| common | 8 | old-global | 0.478 | 0.079 | 5.987 [5.831, 6.057] | 0.493 | 0.098 |
| common | 32 | old-active | 2.589 | 0.367 | 7.039 [6.940, 7.292] | 2.628 | 0.417 |
| common | 32 | old-global | 2.589 | 0.378 | 6.885 [6.820, 7.006] | 2.628 | 0.425 |
| common | 128 | old-active | 18.947 | 2.232 | 8.369 [7.888, 8.726] | 19.080 | 2.393 |
| common | 128 | old-global | 18.947 | 2.752 | 6.714 [6.019, 7.144] | 19.080 | 2.912 |
| distinct | 8 | old-active | 1.082 | 0.095 | 11.416 [10.897, 14.731] | 1.102 | 0.118 |
| distinct | 8 | old-global | 1.082 | 0.081 | 13.122 [10.551, 15.522] | 1.102 | 0.104 |
| distinct | 32 | old-active | 4.984 | 0.340 | 14.435 [13.833, 15.136] | 5.051 | 0.415 |
| distinct | 32 | old-global | 4.984 | 0.310 | 15.982 [15.499, 16.317] | 5.051 | 0.389 |
| distinct | 128 | old-active | 24.780 | 1.220 | 19.981 [19.031, 20.642] | 25.016 | 1.521 |
| distinct | 128 | old-global | 24.780 | 1.374 | 18.384 [17.704, 18.969] | 25.016 | 1.665 |
| failure | 8 | old-active | 0.556 | 0.080 | 6.889 [6.784, 7.458] | 0.568 | 0.095 |
| failure | 8 | old-global | 0.556 | 0.070 | 8.012 [7.032, 8.423] | 0.568 | 0.085 |
| failure | 32 | old-active | 2.464 | 0.277 | 8.743 [8.446, 9.060] | 2.503 | 0.322 |
| failure | 32 | old-global | 2.464 | 0.277 | 8.996 [8.292, 9.388] | 2.503 | 0.323 |
| failure | 128 | old-active | 13.581 | 1.088 | 12.363 [12.278, 12.685] | 13.718 | 1.247 |
| failure | 128 | old-global | 13.581 | 1.204 | 11.039 [10.882, 11.403] | 13.718 | 1.366 |
| common-wide | 8 | old-active | 1.081 | 0.415 | 2.609 [2.566, 2.700] | 1.095 | 0.433 |
| common-wide | 8 | old-global | 1.081 | 0.438 | 2.519 [2.452, 2.572] | 1.095 | 0.456 |
| common-wide | 32 | old-active | 8.543 | 7.020 | 1.220 [1.196, 1.242] | 8.595 | 7.080 |
| common-wide | 32 | old-global | 8.543 | 7.319 | 1.186 [1.143, 1.194] | 8.595 | 7.378 |
| common-wide | 128 | old-active | 95.154 | 171.881 | 0.520 [0.496, 0.541] | 95.406 | 172.091 |
| common-wide | 128 | old-global | 95.154 | 203.360 | 0.466 [0.448, 0.491] | 95.406 | 203.608 |

## Search COW

21 measured repetitions per cell, two warmups. Times in milliseconds; ratios current/older, below 1 favors current. Cold is the additive preparation-charge model described in REPORT.md.

| Regime | n | Control | Current reused ms | Older reused ms | Paired ratio [bootstrap 95%] | Current cold ms | Older cold ms |
|---|---:|---|---:|---:|---:|---:|---:|
| common | 8 | old-active | 0.540 | 0.091 | 5.973 [5.798, 6.184] | 0.558 | 0.108 |
| common | 8 | old-global | 0.540 | 0.088 | 6.103 [5.802, 6.238] | 0.558 | 0.108 |
| common | 32 | old-active | 2.730 | 0.378 | 7.224 [7.074, 7.313] | 2.770 | 0.426 |
| common | 32 | old-global | 2.730 | 0.384 | 7.135 [6.937, 7.213] | 2.770 | 0.432 |
| common | 128 | old-active | 12.830 | 1.549 | 8.296 [8.179, 8.541] | 12.980 | 1.734 |
| common | 128 | old-global | 12.830 | 1.727 | 7.476 [7.229, 7.617] | 12.980 | 1.892 |
| distinct | 8 | old-active | 0.814 | 0.064 | 12.657 [12.526, 14.388] | 0.842 | 0.100 |
| distinct | 8 | old-global | 0.814 | 0.062 | 13.697 [12.563, 14.185] | 0.842 | 0.101 |
| distinct | 32 | old-active | 3.109 | 0.210 | 14.902 [14.637, 15.606] | 3.180 | 0.292 |
| distinct | 32 | old-global | 3.109 | 0.209 | 15.717 [15.261, 16.059] | 3.180 | 0.299 |
| distinct | 128 | old-active | 18.370 | 0.943 | 19.707 [19.277, 20.032] | 18.605 | 1.225 |
| distinct | 128 | old-global | 18.370 | 1.002 | 18.466 [17.752, 18.685] | 18.605 | 1.287 |
| failure | 8 | old-active | 0.392 | 0.054 | 7.307 [7.170, 7.599] | 0.404 | 0.069 |
| failure | 8 | old-global | 0.392 | 0.048 | 8.141 [7.306, 8.338] | 0.404 | 0.066 |
| failure | 32 | old-active | 1.820 | 0.204 | 8.922 [8.664, 9.087] | 1.859 | 0.248 |
| failure | 32 | old-global | 1.820 | 0.204 | 8.898 [8.810, 9.172] | 1.859 | 0.249 |
| failure | 128 | old-active | 9.797 | 0.823 | 12.028 [11.698, 12.391] | 9.990 | 1.004 |
| failure | 128 | old-global | 9.797 | 0.879 | 11.201 [10.765, 11.323] | 9.990 | 1.087 |
| common-wide | 8 | old-active | 0.798 | 0.292 | 2.737 [2.692, 2.763] | 0.813 | 0.310 |
| common-wide | 8 | old-global | 0.798 | 0.298 | 2.671 [2.635, 2.694] | 0.813 | 0.316 |
| common-wide | 32 | old-active | 8.845 | 6.853 | 1.247 [1.216, 1.304] | 8.898 | 6.911 |
| common-wide | 32 | old-global | 8.845 | 6.999 | 1.207 [1.162, 1.277] | 8.898 | 7.060 |
| common-wide | 128 | old-active | 74.710 | 136.275 | 0.522 [0.494, 0.541] | 74.890 | 136.487 |
| common-wide | 128 | old-global | 74.710 | 153.668 | 0.440 [0.428, 0.481] | 74.890 | 154.007 |
