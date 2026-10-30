// Orders: a small file that uses each construct the Kotlin queries read.
package shop.orders

import shop.models.Item
import shop.support.Money as Cash
import shop.util.*

const val RATE = 2

typealias Price = Int

interface Priced : Named {
    fun price(): Price
}

annotation class Audited {
    companion object {
        const val SINCE = 1
    }
}

@Audited
data class Order(val size: Int, private val item: Item?) : Base(size), Priced {
    companion object Factory {
        const val SLOTS = 4
        fun create(): Order = Order(1, null)
    }

    override fun price(): Int {
        item?.pay()
        return Cash.round(fee(size).first) + count(Receipt())
    }
}

enum class Suit { HEARTS, SPADES }

object Registry {
    fun register(order: Order): List<Order> = listOf(order)
}

fun fee(cents: Int): Pair<Int, Int> {
    return cents / 100 to cents % 100
}

class FeeTest {
    @Test
    fun rounds() {
        assertEquals(0, fee(1).first)
    }
}
