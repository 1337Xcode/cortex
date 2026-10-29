// Orders: a small file that uses each construct the Java queries read.
package shop;

import java.util.List;
import java.util.*;
import static org.junit.Assert.assertEquals;

@Deprecated
public class Order extends Base<Item> implements Priced, Comparable<Order> {
    static final int LIMIT = 3;
    private final String name;

    public Order(String name) throws IOException {
        super(name);
        this.name = name;
    }

    @Override
    public Money price(String currency) {
        return new Money(total(), currency);
    }

    int total() {
        return items().size();
    }

    @Test
    void pays() {
        assertEquals(3, total());
    }
}

interface Priced extends Named {
    int RATE = 2;
    Money price(String currency);
}

enum Status { OPEN, PAID }

record Line(Item item, int count) {}

@interface Audit {}
