# Technical (中文版)



# TODO

- 文档和单测
- 性能测试
- Sized 版本?
- unsafe 注解
- FdWithExtra
- (Interest, AsFd, )
- interest.remove_xxx

# 提纲

1. 引子
    1. 跳过所有开销, 性能 MAX, 易用易测试
2. 与 mio, EventManager 对比
    1. EventManager 相比 mio 能把事件处理从静态的 match 代码结构分解到运行时灵活的注册修改, 这很好, 大型工程刚需
    2. 但每次触发事件, 热路径上平添两次 HashMap 查询开销
    3. 更糟的是, std HashMap 使用抗 DDOS 哈希算法, 更慢
    4. 洞察
        1. Linux 提供给用户用来分辨事件的 epoll data 恰好是 64bits
        2. 直接将其视为对象地址, callq 直达对象方法, 零额外开销, 甚至比 mio 还少一次分支指令
3. 胖指针问题
    1. 怎么找到对象的成员函数地址? 编译时多态当然可以, 但最广泛使用的运行时多态, 也就是 trait 对象怎么办?
    2. Rust trait 对象内存布局 对比 C++ 单继承多态对象内存布局, 胖指针 16 字节放不下
    3. 洞察
        1. 运行时计算 Layout, 亲自操刀分配回收, 把 vtable ptr 放进对象"内部"
            1. 首先排除例外: 非 64 位平台, Rust ZST
            2. 编译时断言胖指针布局, 防止前提不成立
            3. 施展黑魔法: Reinterprets 胖指针, 获取虚表地址, 计算新结构体布局
                1. 细小但却致命: 大于 8 字节的 align 空隙问题 (配图)
                2. 黑魔法进阶: align 一定是所有成员的要求的最大值, vptr 紧贴数据
        2. 在解引用和 drop 如何正确处理
4. 所有权问题
    1. Subscriber 的 handle 方法获取了双重可变引用
    2. 在黑魔法中越陷越深:
        1. 牢记在心: [The Problem With Single-threaded Shared Mutability](https://manishearth.github.io/blog/2015/05/17/the-problem-with-shared-mutability/)
        2. 我们知道可以从 RefCell 获取单线程共享可变性
            1. 这样即便从 &Eventp 也可以取出 &mut Subscriber
            2. 如此依赖, 问题分解为两步
                1. 我们能不能用形式化语言证明, 如果它是个 RefCell 包裹的, ref count 是不是不可能大于 1
                2. 在 Subscriber.handle 调用期间, &mut Eventp 的修改能力是不是被规约到 &Eventp
                    1. 公共方法不会
                    2. 无公共字段可修改
                    3. 整体替换
                        1. 好吧, 我必须承认, 我们击穿了 rust 类型系统, 是时候召唤黑魔王了: `Pin<&mut Eventp>`

缺陷: 现阶段无法使 Eventp 变成 Send: Eventp->ThinBoxSubscriber->
