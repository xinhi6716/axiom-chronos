## base
PURPOSE base
PATHS lib/
DEPENDS none
CONTRACT
  base contract
ACCEPT lib/check test
## app
PURPOSE app
PATHS src/ future/
DEPENDS base
CONTRACT
  app contract
ACCEPT src/check test
