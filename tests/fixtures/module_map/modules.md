## a
PURPOSE alpha
PATHS src/a/
DEPENDS none
CONTRACT
  alpha contract 中文
ACCEPT tests/a, `cargo test --test set_x`
SERVES A
## ab
PURPOSE beta
PATHS src/ab/
DEPENDS none
CONTRACT
  beta secret
ACCEPT tests/ab, `cargo test`
SERVES B
## c
PURPOSE gamma
PATHS src/c/
DEPENDS none
CONTRACT
  gamma secret
ACCEPT tests/c, `cargo test`
SERVES C
