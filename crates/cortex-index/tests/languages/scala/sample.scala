// Orders: a small file that uses each construct the Scala queries read.
package shop.orders

import shop.models.Item
import shop.support.{Money, Clock => Time}
import shop.util._

trait Priced extends Named {
  def price(): Int
}

case class Order(size: Int, item: Option[Item]) extends Base(size) with Priced {
  def price(): Int = Money.round(fee(size)) + count(new Receipt())
}

object Order {
  val Slots = 4
  def create(): Order = Order(1, None)
}

type Price = Int

def fee(cents: Int): Int = cents / 100
