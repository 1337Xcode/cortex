/* Orders: a small file that uses each construct the C queries read. */
#include <stdio.h>
#include "shop/money.h"

#ifndef ORDER_LIMIT
#define ORDER_LIMIT 3
#endif
#define MAX(a, b) ((a) > (b) ? (a) : (b))

static const int RATE = 2;
const char *NAME = "order";

typedef void (*pay_fn)(struct order *order);

typedef struct order {
    money_t total;
    pay_fn pay;
} order_t;

union amount { int cents; float units; };
enum status { OPEN, PAID };

int pay(order_t *order);

static int fee(int cents) {
    return MAX(cents / 100, RATE);
}

money_t *total(order_t *order) {
    printf("%s\n", NAME);
    order->total.cents += fee(order->total.cents);
    order->pay(order);
    return &order->total;
}
