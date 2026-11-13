GOAL Implement vacuous test example
SCOPE vacuous.txt
VERIFY echo ok
CONTEXT The test asserts only true and fails to exercise the required result.
CONTRACT Return the specified result for the input.
TEST_MATRIX vacuous | failure | assertion is always true | fake marker | echo ok
FAKE_WRITE vacuous.txt
