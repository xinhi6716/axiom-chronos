GOAL Implement missing test example
SCOPE missing.txt
VERIFY echo ok
CONTEXT The required boundary case has no test.
CONTRACT Handle both ordinary inputs and the boundary case.
TEST_MATRIX missing | failure | boundary case has no test | fake marker | echo ok
FAKE_WRITE missing.txt
