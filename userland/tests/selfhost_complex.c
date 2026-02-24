/* selfhost_complex.c -- Complex self-hosting compilation test
 *
 * Tests libc functionality: stdio, stdlib, string, memory allocation.
 * Compiled ON VeridianOS to verify the full toolchain works with
 * non-trivial C programs.
 *
 * Expected output: "COMPLEX_SELFHOST_PASS"
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Simple linked list to test malloc/free */
struct node {
    int value;
    struct node *next;
};

static struct node *list_push(struct node *head, int val)
{
    struct node *n = (struct node *)malloc(sizeof(struct node));
    if (!n) return head;
    n->value = val;
    n->next = head;
    return n;
}

static int list_sum(struct node *head)
{
    int sum = 0;
    while (head) {
        sum += head->value;
        head = head->next;
    }
    return sum;
}

/* Simple string test */
static int test_strings(void)
{
    char buf[128];
    strcpy(buf, "Hello");
    strcat(buf, ", ");
    strcat(buf, "VeridianOS");
    if (strlen(buf) != 17) return 0;
    if (strcmp(buf, "Hello, VeridianOS") != 0) return 0;

    /* memcpy / memset */
    char src[] = "ABCDEFGH";
    char dst[16];
    memcpy(dst, src, 9);
    if (memcmp(src, dst, 9) != 0) return 0;

    memset(dst, 'X', 4);
    if (dst[0] != 'X' || dst[3] != 'X' || dst[4] != 'E') return 0;

    return 1;
}

/* Arithmetic test */
static int test_arithmetic(void)
{
    /* Fibonacci */
    int a = 0, b = 1;
    for (int i = 0; i < 20; i++) {
        int tmp = a + b;
        a = b;
        b = tmp;
    }
    /* fib(20) = 6765, fib(21) = 10946; a=fib(20), b=fib(21) */
    if (a != 6765) return 0;

    /* Integer division and modulus */
    if (12345 / 67 != 184) return 0;
    if (12345 % 67 != 17) return 0;

    return 1;
}

/* Sorting test */
static int cmp_int(const void *a, const void *b)
{
    int ia = *(const int *)a;
    int ib = *(const int *)b;
    return (ia > ib) - (ia < ib);
}

static int test_sort(void)
{
    int arr[] = {42, 7, 99, 1, 23, 56, 3, 88, 15, 67};
    int n = sizeof(arr) / sizeof(arr[0]);
    qsort(arr, n, sizeof(int), cmp_int);

    /* Verify sorted */
    for (int i = 1; i < n; i++) {
        if (arr[i] < arr[i - 1]) return 0;
    }
    if (arr[0] != 1 || arr[n - 1] != 99) return 0;

    return 1;
}

int main(void)
{
    int pass = 1;

    /* Test 1: Strings */
    if (!test_strings()) {
        write(1, "FAIL: strings\n", 14);
        pass = 0;
    }

    /* Test 2: Arithmetic */
    if (!test_arithmetic()) {
        write(1, "FAIL: arithmetic\n", 17);
        pass = 0;
    }

    /* Test 3: malloc + linked list */
    struct node *list = NULL;
    for (int i = 1; i <= 100; i++)
        list = list_push(list, i);
    if (list_sum(list) != 5050) {
        write(1, "FAIL: malloc/list\n", 18);
        pass = 0;
    }

    /* Test 4: qsort */
    if (!test_sort()) {
        write(1, "FAIL: qsort\n", 12);
        pass = 0;
    }

    /* Test 5: snprintf */
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%d + %d = %d", 17, 25, 42);
    if (n != 12 || strcmp(buf, "17 + 25 = 42") != 0) {
        write(1, "FAIL: snprintf\n", 15);
        pass = 0;
    }

    /* Test 6: atoi / strtol */
    if (atoi("12345") != 12345 || atoi("-99") != -99) {
        write(1, "FAIL: atoi\n", 11);
        pass = 0;
    }

    if (pass) {
        write(1, "COMPLEX_SELFHOST_PASS\n", 22);
    } else {
        write(1, "COMPLEX_SELFHOST_FAIL\n", 22);
    }

    return pass ? 0 : 1;
}
