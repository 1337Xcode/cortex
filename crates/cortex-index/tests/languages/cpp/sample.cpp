// Orders: a small file that uses each construct the C++ queries read.
#include <vector>
#include "shop/money.h"

namespace shop {

constexpr int LIMIT = 3;
using money = long;
enum class Status { Open, Paid };

template <typename T>
class Ledger : public Base<T>, private Audit {
public:
    explicit Ledger(int size) : size_(size) {}
    ~Ledger() {}
    bool operator==(const Ledger& other) const;
    void add(const T& entry) { entries_.push_back(entry); }
    static constexpr int SLOTS = 4;

private:
    std::vector<T> entries_;
    int size_;
};

money Order::total() const {
    return std::max(fee(LIMIT), helper<money>(1));
}

}  // namespace shop

TEST(OrderTest, Pays) {
    auto* order = new shop::Order();
    EXPECT_EQ(order->total(), 3);
}
