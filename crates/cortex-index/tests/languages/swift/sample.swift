// Orders: a small file that uses each construct the Swift queries read.
import Foundation
import struct Shop.Money

let rate = 2

typealias Price = Int

protocol Priced: Named {
    associatedtype Unit
    func price() -> Price
}

@objc final class Order: Base, Priced {
    static let slots = 4
    private var item: Item?

    init(size: Int) {
        super.init()
        log("made")
    }

    func price() -> Int {
        item?.pay()
        return Money.round(fee(size)) + count(Receipt())
    }
}

extension Order: Codable {
    func extra() -> Int { 1 }
}

func fee(_ cents: Int) -> Int {
    return cents / 100
}
