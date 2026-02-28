#!/bin/sh
# test_arithmetic.sh -- Tests for arithmetic evaluation in vsh
#
# Covers: $(( )), let, arithmetic for loops, operators (+, -, *, /, %,
#         **, comparison, bitwise, logical), ternary, parentheses,
#         hex/octal literals, variable references in arithmetic.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Arithmetic Tests ===\n\n"

# ---------------------------------------------------------------------------
# Basic arithmetic: $(( ))
# ---------------------------------------------------------------------------

_TEST_NAME="arith: addition"
_out=$($VSH -c 'echo $((2 + 3))')
assert_equal "$_out" "5"

_TEST_NAME="arith: subtraction"
_out=$($VSH -c 'echo $((10 - 3))')
assert_equal "$_out" "7"

_TEST_NAME="arith: multiplication"
_out=$($VSH -c 'echo $((4 * 5))')
assert_equal "$_out" "20"

_TEST_NAME="arith: division"
_out=$($VSH -c 'echo $((15 / 3))')
assert_equal "$_out" "5"

_TEST_NAME="arith: modulo"
_out=$($VSH -c 'echo $((17 % 5))')
assert_equal "$_out" "2"

_TEST_NAME="arith: exponentiation"
_out=$($VSH -c 'echo $((2 ** 10))')
assert_equal "$_out" "1024"

_TEST_NAME="arith: exponentiation base 3"
_out=$($VSH -c 'echo $((3 ** 4))')
assert_equal "$_out" "81"

_TEST_NAME="arith: negative result"
_out=$($VSH -c 'echo $((3 - 10))')
assert_equal "$_out" "-7"

_TEST_NAME="arith: zero"
_out=$($VSH -c 'echo $((0))')
assert_equal "$_out" "0"

_TEST_NAME="arith: unary minus"
_out=$($VSH -c 'echo $((-5))')
assert_equal "$_out" "-5"

_TEST_NAME="arith: unary plus"
_out=$($VSH -c 'echo $((+5))')
assert_equal "$_out" "5"

_TEST_NAME="arith: no spaces"
_out=$($VSH -c 'echo $((2+3))')
assert_equal "$_out" "5"

_TEST_NAME="arith: extra spaces"
_out=$($VSH -c 'echo $((  2  +  3  ))')
assert_equal "$_out" "5"

_TEST_NAME="arith: large number"
_out=$($VSH -c 'echo $((1000000 * 1000))')
assert_equal "$_out" "1000000000"

# ---------------------------------------------------------------------------
# Operator precedence
# ---------------------------------------------------------------------------

_TEST_NAME="arith: precedence mul before add"
_out=$($VSH -c 'echo $((2 + 3 * 4))')
assert_equal "$_out" "14"

_TEST_NAME="arith: precedence div before sub"
_out=$($VSH -c 'echo $((10 - 6 / 2))')
assert_equal "$_out" "7"

_TEST_NAME="arith: parentheses override precedence"
_out=$($VSH -c 'echo $(((2 + 3) * 4))')
assert_equal "$_out" "20"

_TEST_NAME="arith: nested parentheses"
_out=$($VSH -c 'echo $(((2 + 3) * (4 + 1)))')
assert_equal "$_out" "25"

_TEST_NAME="arith: complex expression"
_out=$($VSH -c 'echo $((2 * 3 + 4 * 5))')
assert_equal "$_out" "26"

_TEST_NAME="arith: exponent right-associative"
_out=$($VSH -c 'echo $((2 ** 3 ** 2))')
# 2 ** (3 ** 2) = 2 ** 9 = 512
assert_equal "$_out" "512"

# ---------------------------------------------------------------------------
# Comparison operators
# ---------------------------------------------------------------------------

_TEST_NAME="arith: == true"
_out=$($VSH -c 'echo $((5 == 5))')
assert_equal "$_out" "1"

_TEST_NAME="arith: == false"
_out=$($VSH -c 'echo $((5 == 3))')
assert_equal "$_out" "0"

_TEST_NAME="arith: != true"
_out=$($VSH -c 'echo $((5 != 3))')
assert_equal "$_out" "1"

_TEST_NAME="arith: != false"
_out=$($VSH -c 'echo $((5 != 5))')
assert_equal "$_out" "0"

_TEST_NAME="arith: < true"
_out=$($VSH -c 'echo $((3 < 5))')
assert_equal "$_out" "1"

_TEST_NAME="arith: < false"
_out=$($VSH -c 'echo $((5 < 3))')
assert_equal "$_out" "0"

_TEST_NAME="arith: > true"
_out=$($VSH -c 'echo $((5 > 3))')
assert_equal "$_out" "1"

_TEST_NAME="arith: > false"
_out=$($VSH -c 'echo $((3 > 5))')
assert_equal "$_out" "0"

_TEST_NAME="arith: <= true (equal)"
_out=$($VSH -c 'echo $((5 <= 5))')
assert_equal "$_out" "1"

_TEST_NAME="arith: <= true (less)"
_out=$($VSH -c 'echo $((3 <= 5))')
assert_equal "$_out" "1"

_TEST_NAME="arith: <= false"
_out=$($VSH -c 'echo $((6 <= 5))')
assert_equal "$_out" "0"

_TEST_NAME="arith: >= true (equal)"
_out=$($VSH -c 'echo $((5 >= 5))')
assert_equal "$_out" "1"

_TEST_NAME="arith: >= true (greater)"
_out=$($VSH -c 'echo $((7 >= 5))')
assert_equal "$_out" "1"

_TEST_NAME="arith: >= false"
_out=$($VSH -c 'echo $((4 >= 5))')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# Bitwise operators
# ---------------------------------------------------------------------------

_TEST_NAME="arith: bitwise AND"
_out=$($VSH -c 'echo $((12 & 10))')
assert_equal "$_out" "8"

_TEST_NAME="arith: bitwise OR"
_out=$($VSH -c 'echo $((12 | 10))')
assert_equal "$_out" "14"

_TEST_NAME="arith: bitwise XOR"
_out=$($VSH -c 'echo $((12 ^ 10))')
assert_equal "$_out" "6"

_TEST_NAME="arith: bitwise NOT"
_out=$($VSH -c 'echo $((~0))')
assert_equal "$_out" "-1"

_TEST_NAME="arith: bitwise NOT of 1"
_out=$($VSH -c 'echo $((~1))')
assert_equal "$_out" "-2"

_TEST_NAME="arith: left shift"
_out=$($VSH -c 'echo $((1 << 8))')
assert_equal "$_out" "256"

_TEST_NAME="arith: right shift"
_out=$($VSH -c 'echo $((256 >> 4))')
assert_equal "$_out" "16"

_TEST_NAME="arith: shift by 0"
_out=$($VSH -c 'echo $((42 << 0))')
assert_equal "$_out" "42"

# ---------------------------------------------------------------------------
# Logical operators
# ---------------------------------------------------------------------------

_TEST_NAME="arith: logical AND true"
_out=$($VSH -c 'echo $((1 && 1))')
assert_equal "$_out" "1"

_TEST_NAME="arith: logical AND false"
_out=$($VSH -c 'echo $((1 && 0))')
assert_equal "$_out" "0"

_TEST_NAME="arith: logical OR true"
_out=$($VSH -c 'echo $((0 || 1))')
assert_equal "$_out" "1"

_TEST_NAME="arith: logical OR false"
_out=$($VSH -c 'echo $((0 || 0))')
assert_equal "$_out" "0"

_TEST_NAME="arith: logical NOT true"
_out=$($VSH -c 'echo $((!0))')
assert_equal "$_out" "1"

_TEST_NAME="arith: logical NOT false"
_out=$($VSH -c 'echo $((!1))')
assert_equal "$_out" "0"

_TEST_NAME="arith: logical NOT of nonzero"
_out=$($VSH -c 'echo $((!42))')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# Ternary operator
# ---------------------------------------------------------------------------

_TEST_NAME="arith: ternary true"
_out=$($VSH -c 'echo $((1 ? 10 : 20))')
assert_equal "$_out" "10"

_TEST_NAME="arith: ternary false"
_out=$($VSH -c 'echo $((0 ? 10 : 20))')
assert_equal "$_out" "20"

_TEST_NAME="arith: ternary with expression"
_out=$($VSH -c 'echo $((5 > 3 ? 100 : 200))')
assert_equal "$_out" "100"

_TEST_NAME="arith: ternary with computation"
_out=$($VSH -c 'echo $((1 ? 2+3 : 4+5))')
assert_equal "$_out" "5"

# ---------------------------------------------------------------------------
# Variable references in arithmetic
# ---------------------------------------------------------------------------

_TEST_NAME="arith: variable reference"
_out=$($VSH -c 'X=10; echo $((X + 5))')
assert_equal "$_out" "15"

_TEST_NAME="arith: multiple variables"
_out=$($VSH -c 'A=3; B=7; echo $((A + B))')
assert_equal "$_out" "10"

_TEST_NAME="arith: unset variable is 0"
_out=$($VSH -c 'echo $((UNDEFINED_VAR_XYZ + 5))')
assert_equal "$_out" "5"

_TEST_NAME="arith: variable with \$"
_out=$($VSH -c 'X=10; echo $(($X + 5))')
assert_equal "$_out" "15"

_TEST_NAME="arith: variable multiplication"
_out=$($VSH -c 'W=5; H=3; echo $((W * H))')
assert_equal "$_out" "15"

# ---------------------------------------------------------------------------
# Numeric literals
# ---------------------------------------------------------------------------

_TEST_NAME="arith: hex literal"
_out=$($VSH -c 'echo $((0xFF))')
assert_equal "$_out" "255"

_TEST_NAME="arith: hex uppercase"
_out=$($VSH -c 'echo $((0XFF))')
assert_equal "$_out" "255"

_TEST_NAME="arith: hex small"
_out=$($VSH -c 'echo $((0x10))')
assert_equal "$_out" "16"

_TEST_NAME="arith: octal literal"
_out=$($VSH -c 'echo $((010))')
assert_equal "$_out" "8"

_TEST_NAME="arith: octal 077"
_out=$($VSH -c 'echo $((077))')
assert_equal "$_out" "63"

_TEST_NAME="arith: decimal zero"
_out=$($VSH -c 'echo $((0))')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# let builtin
# ---------------------------------------------------------------------------

_TEST_NAME="let: nonzero result returns 0"
assert_exit_code 0 $VSH -c 'let "5+3"'

_TEST_NAME="let: zero result returns 1"
assert_exit_code 1 $VSH -c 'let "0"'

_TEST_NAME="let: expression evaluation"
_out=$($VSH -c 'let "10-5"; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="let: zero expression"
_out=$($VSH -c 'let "5-5"; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="let: complex expression"
_out=$($VSH -c 'let "2**10"; echo $?')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# Arithmetic evaluation (( ))
# ---------------------------------------------------------------------------

_TEST_NAME="(( )): nonzero is success"
assert_exit_code 0 $VSH -c '(( 42 ))'

_TEST_NAME="(( )): zero is failure"
assert_exit_code 1 $VSH -c '(( 0 ))'

_TEST_NAME="(( )): comparison true"
assert_exit_code 0 $VSH -c '(( 5 > 3 ))'

_TEST_NAME="(( )): comparison false"
assert_exit_code 1 $VSH -c '(( 3 > 5 ))'

_TEST_NAME="(( )): with variable"
assert_exit_code 0 $VSH -c 'X=10; (( X > 5 ))'

_TEST_NAME="(( )): in if condition"
_out=$($VSH -c 'if (( 2 + 2 == 4 )); then echo correct; fi')
assert_equal "$_out" "correct"

_TEST_NAME="(( )): complex in if"
_out=$($VSH -c 'X=10; if (( X % 2 == 0 )); then echo even; else echo odd; fi')
assert_equal "$_out" "even"

# ---------------------------------------------------------------------------
# Arithmetic in variable assignment
# ---------------------------------------------------------------------------

_TEST_NAME="arith assign: X=\$((expr))"
_out=$($VSH -c 'X=$((2+3)); echo $X')
assert_equal "$_out" "5"

_TEST_NAME="arith assign: chained"
_out=$($VSH -c 'A=$((2+3)); B=$((A*2)); echo $B')
assert_equal "$_out" "10"

_TEST_NAME="arith assign: in loop"
_out=$($VSH -c 'sum=0; for i in 1 2 3 4 5; do sum=$((sum+i)); done; echo $sum')
assert_equal "$_out" "15"

_TEST_NAME="arith assign: decrement"
_out=$($VSH -c 'X=10; X=$((X-1)); echo $X')
assert_equal "$_out" "9"

# ---------------------------------------------------------------------------
# Comma operator
# ---------------------------------------------------------------------------

_TEST_NAME="arith: comma returns last"
_out=$($VSH -c 'echo $((1, 2, 3))')
assert_equal "$_out" "3"

_TEST_NAME="arith: comma evaluates all"
_out=$($VSH -c 'echo $((10+5, 20+30))')
assert_equal "$_out" "50"

# ---------------------------------------------------------------------------
# Division edge cases
# ---------------------------------------------------------------------------

_TEST_NAME="arith: integer division truncates"
_out=$($VSH -c 'echo $((7 / 2))')
assert_equal "$_out" "3"

_TEST_NAME="arith: negative integer division"
_out=$($VSH -c 'echo $((-7 / 2))')
assert_equal "$_out" "-3"

_TEST_NAME="arith: modulo with negative"
_out=$($VSH -c 'echo $((-7 % 3))')
assert_equal "$_out" "-1"

_TEST_NAME="arith: power of 0"
_out=$($VSH -c 'echo $((5 ** 0))')
assert_equal "$_out" "1"

_TEST_NAME="arith: 0 to power"
_out=$($VSH -c 'echo $((0 ** 5))')
assert_equal "$_out" "0"

_TEST_NAME="arith: 1 to any power"
_out=$($VSH -c 'echo $((1 ** 100))')
assert_equal "$_out" "1"

report
