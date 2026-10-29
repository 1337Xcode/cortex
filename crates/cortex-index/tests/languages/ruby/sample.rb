# Orders: a small file that uses each construct the Ruby queries read.
require "json"
require_relative "../lib/money"
autoload :Receipt, "shop/receipt"
require File.join(__dir__, "tax")

module Shop
  LIMIT = 3
  Line = Struct.new(:item, :count)

  class Order < Base
    include Comparable
    extend Audit::Logging

    def initialize(items)
      @items = items
      count = items.size
      log(count)
    end

    def self.build(*lines)
      new(lines.map { |line| line.item })
    end

    def total=(value)
      @total = value
    end

    def paid?
      total > LIMIT && Receipt.valid?(self)
    end

    alias cost total
  end
end

def checkout(order)
  order.total = JSON.parse(order.to_s)
  receipt = Shop::Receipt.new(order)
  receipt
end
