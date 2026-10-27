use thrum_engine::{ErrType, vm_compiling::VmValue};
mod common;


#[test] fn math() {
    test!("1+2 * 3", VmValue::Int(7));
    test!("(1 + 2)*3", VmValue::Int(9));
    test!("1 + 2 * 3 +9/ 3", VmValue::Int(10));
    test!("6--21 * 3", VmValue::Int(69));
    test!("5 % 2", VmValue::Int(1));
}

#[test] fn float_math() {
    test!("1.5 + 2.5 * 2.0", VmValue::Float(6.5));
    test!("(10.0 - 2.5) / 2.5", VmValue::Float(3.0));
    test!("5.5 % 2.0", VmValue::Float(1.5));
    test!("-3.5 + 1.0", VmValue::Float(-2.5));
    test!("------4.5", VmValue::Float(4.5));
}

#[test] fn int_comparisons() {
    test!("1 < 2", VmValue::Bool(true));
    test!("6 != 9", VmValue::Bool(true));
}
#[test] fn string_comparisons() {
    test!("\"a\" < \"b\"", VmValue::Bool(true));
}
#[test] fn float_comparisons() {
    test!("1.5 < 2.5", VmValue::Bool(true));
    test!("3.5 > 5.5", VmValue::Bool(false));
    test!("2.25 == 2.25", VmValue::Bool(true));
    test!("2.25 != 2.5", VmValue::Bool(true));
}

#[test] fn short_circuiting() {
    test!(r#"
        true or panic("OR short-circuit failed...")
    "#, VmValue::Bool(true));
    test!(r#"
        false and panic("AND short-circuit failed...")
    "#, VmValue::Bool(false));

    test!(r#"
        let t1 = (10, 20)
        let t2 = (10, 20)

        let res1 = t1^ is (10, _) or panic("OR short-circuit failed...")
        let res2 = t2^ is (99, _) and panic("AND short-circuit failed...")

        res1 and !res2
    "#, VmValue::Bool(true));
}


#[test] fn assignments() {
    test!("
        let mut x = 10
        x += 5   // 15
        x *= 2   // 30
        x -= 6   // 24
        x /= 3   // 8
        x %= 5   // 3
        x^
    ", VmValue::Int(3));

    test!("
        let mut a = 1
        let mut b = 2
        (a, b) = (b, a)
        a * 10 + b
    ", VmValue::Int(21));

    test!("
        let mut a = 1
        (_, a) = (a, 5*a)
        a^
    ", VmValue::Int(5));

    test!("let mut x = 5; x *= 2; x *= x; x", VmValue::Int(100));
}

#[test] fn scoping() {
    test!("let x = 50; x^", VmValue::Int(50));

    test!("
        let x = 12
        let res = { let x = 33; x + 5 }
        res + x
    ", VmValue::Int(50));

    test!("
        let x = 1
        let res = {
            let x = 2
            {
                let x = 3
                x * 10
            }
            + x * 100
        }
        res + x
    ", VmValue::Int(231)); // 200 + 30 + 1

    test!("
        { #bloc
            break #bloc 50
            1
        }
    ", VmValue::Int(50));

    test!("{ #label }", VmValue::Void);
    test!("{}", VmValue::Void);
}

#[test] fn delayed_let() {
    test!("
        { #bloc
            let x
            if true { x = 5 }
            else break #bloc -1
            x^
        }
    ", VmValue::Int(5));

    test!("let a;         (a, let b) = (1, 2); a + b", VmValue::Int(3));
    test!("let mut a = 5; (a, let b) = (1, 2); a + b", VmValue::Int(3));
}


#[test] fn loop_shenanigance() {
    test!("
        let mut i = 0
        while i < 5 {
            i += 1
        }
        i^
    ", VmValue::Int(5));

    test!("
        let mut sum = 0
        let mut i = 0
        loop {
            i += 1
            if i > 5 { break }
            if i^ is 3 { continue }
            sum += i
        }
        sum^  // 1 + 2 + 4 + 5 = 12
    ", VmValue::Int(12));

    test!("
        let mut res = 1
        loop #outer {
            res += 1
            loop {
                res += 22
                if res < 30 { continue #outer }
                if res < 50 { continue } // inner
                break #outer res^
            }
            break -1
        }
    ", VmValue::Int(69));

    test!("loop { break #loop }", VmValue::Void);
}

#[test] fn diabolical_loops() {
    test!("1 + loop { 1 + break 1 }", VmValue::Int(2));

    test!("
        let mut i = 1

        1 + loop {
            i += 1
            if i < 3 {
                (1, 1, 1, continue)
            }
            (1, 1, 1, break 1)
        }
    ", VmValue::Int(2));
}

#[test] fn for_loop() {
    test!("
        let mut sum = 0;
        for x in 0..10 => sum += x
        sum^
    ", VmValue::Int(45));

    test!("
        let mut sum = 0;
        for x in 0..=10 => sum += x
        sum^
    ", VmValue::Int(55));

    test!("let mut sum = 0; for x in 10..10 => sum = x^; sum^", VmValue::Int(0));
    test!("let mut sum = 0; for x in 10..=10 => sum = x^; sum^", VmValue::Int(10));
}

#[test] fn tuples() {
    test!("(x: 10).x", VmValue::Int(10));
    test!("(x: 10,).x", VmValue::Int(10));
}

#[test] fn tup_arrays() {
    test!("
        let x = (1, 2, 3)
        x.0 + x.1 * x[2]
    ", VmValue::Int(7));

    test!("
        let mut x = (9; 30)
        x.2 %= 5
        x[2] %= 3
        x[2]^
    ", VmValue::Int(1));

    test!("const LENGTH = 10; (0; LENGTH)", VmValue::Tup(vec![VmValue::Int(0); 10]));

    test!("
        let mut grid = ((1, 2), (3, 4))
        grid[1][0] = 99
        grid[0][1] + grid[1][0]
    ", VmValue::Int(101)); // 2 + 99
}

#[test] fn some_strings() {
    test!(r#"
        let s = "klaus"
        let arr = (s, "x")
        arr[0]^
    "#, VmValue::OwnedStr("klaus".to_string()));

    test!(r#"  "abcd".len()  "#, VmValue::Int(4));
    test!(r#"  "🦀".len()  "#, VmValue::Int(4));
    test!(r#"  "ê".len()  "#, VmValue::Int(2));
}

#[test] fn formatting_strings() {
    test!(r#"
        let piece = "orl"
        "hello w{piece}d!"
    "#, VmValue::OwnedStr("hello world!".to_string()));

    test!(r#"
        let x = 3-2
        let xy = !true
        let y = 69.08 / 22.0
        let z = "nested"
        "hello {x}, {xy} and {y}, even works {z}!"
    "#, VmValue::OwnedStr("hello 1, false and 3.14, even works nested!".to_string())
    );

    test!(r#"
        let a = 1
        let b = 2
        "{a}{b}{a}{a}"
    "#, VmValue::OwnedStr("1211".to_string()));

    test!(r#"
        let x = 5
        let y = 3.3
        if true { "x: {x}, y: {y}" }
        else { "y: {y}, x: {x}" }
    "#, VmValue::OwnedStr("x: 5, y: 3.3".to_string()));
    test!(r#"
        let x = 5
        let y = 3.3
        if false { "x: {x}, y: {y}" }
        else { "y: {y}, x: {x}" }
    "#, VmValue::OwnedStr("y: 3.3, x: 5".to_string()));
}

#[test] fn new_line_iss() {
    test!("3 \n is 3", VmValue::Bool(true));
    test!("if 3 \n is 3 => true else false", VmValue::Bool(true));

    test!("
        match 16
        is 5 {
            1
            is 2
        }
        is 4 => (
            1
            is 2
        )
        is _ => true
    ", VmValue::Bool(true));
}


#[test] fn ensure_expr() {
    test!("ensure true else panic(\"OoH No!!\"); 4", VmValue::Int(4));
    test_err!("ensure false else panic(\"OoH No!!\")", ErrType::RuntimeError { .. });
    test_err!("ensure true else print(\"oOh nO!!\")", ErrType::TyperMismatch { .. });
}

#[test] fn slice_coercion_and_indexing() {
    test!("
        let arr = (10, 20, 30)
        let s: &[int] = arr
        s[1]^
    ", VmValue::Int(20));

    test!("
        let arr = (10, 20, 30)
        let s: &[int] = arr
        s.len()
    ", VmValue::Int(3));

    test!("
        let mut arr = (10, 20, 30)
        let s: &mut [int] = mut arr
        s[1] = 99
        arr[1]^
    ", VmValue::Int(99));

    test!("
        let mut arr = (10, 20, 30)
        let s: &mut [int] = mut arr
        s[0] += 5
        s[1] *= 2
        s[2] -= s[0]
        arr[0] + arr[1] + arr[2]
    ", VmValue::Int(70));
}

#[test] fn slice_empty() {
    test!("
        let arr = (0; 0)
        let s: &[int] = arr
        s.len()
    ", VmValue::Int(0));
}

#[test] fn slice_of_tuples() {
    test!("
        let grid = ((1, 2), (3, 4), (5, 6))
        let s: &[(int, int)] = grid
        s[0].0 + s[1].1 + s[2].0
    ", VmValue::Int(1 + 4 + 5));
}

#[test] fn slice_function_arguments() {
    test!("
        fn sum(s: &[int]) -> int {
            let mut total = 0
            for i in 0..s.len() {
                total += s[i]
            }
            total
        }
        let arr = (1, 2, 3, 4, 5)
        sum(arr)
    ", VmValue::Int(15));

    test!("
        fn mutate_first(s: &mut [int]) {
            s[0] = 775
        }
        let mut data = (1, 2)
        mutate_first(mut data)
        data[0] + data[1]
    ", VmValue::Int(777));

    test!("
        fn inner(s: &[int]) -> int => s[1]
        fn outer(s: &[int]) -> int => inner(s) + s[0]
        let arr = (100, 20)
        outer(arr)
    ", VmValue::Int(120));
}


#[test] fn functions() {
    test!("
        fn fun() -> int => 4
        fun()
    ", VmValue::Int(4));

    test!("
        fn fun(x: int, y: int) -> int {
            x * y + x
        }
        fun(2, 4)
    ", VmValue::Int(10));


    test!(r#"
        fn test(tup: (int, int), expected_bool: bool) {
            let maybe = { tup^ is let (x, 0) and x > 10 }
            if maybe^ is !expected_bool^ {
                panic("nope.")
            }
        }

        test((30, 1), false)
        test((3, 0), false)
        test((30, 0), true)
    "#, VmValue::Void);
}

#[test] fn recursion() {
    test!("
        fn fib(n: int) -> int {
            if n < 2 { return n }
            return fib(n-1) + fib(n-2)
        }
        fib(8)
    ", VmValue::Int(21));
}



#[test] fn single_tuple_type_instantiation() {
    test!("type X = int;         -X{ 2 }", VmValue::Int(-2));
    test!("type X = float;       -X{ 2.0 }", VmValue::Float(-2.0));
    test!("type X = (int,);      X{ 2 }", VmValue::Tup(vec![VmValue::Int(2)]));
    test!("type X = (int, bool); X{ 2, true }", VmValue::Tup(vec![VmValue::Int(2), VmValue::Bool(true)]));
    test!("type X = ();          X{ }", VmValue::Tup(vec![]));
}

#[test] fn single_tuple_type_destruction() {
    test!("type X = int;              let X{ a } = X{ 2 };                 a^",     VmValue::Int(2));
    test!("type X = (int,);           let X{ a } = X{ 2 };                 a^",     VmValue::Int(2));
    test!("type X = (int, int);       let X{ a, b } = X{ 3, 4 };           a * b", VmValue::Int(12));
    test!("type X = (x: int, y: int); let X{ x: a, y: } = X{ x: 3, y: 4 }; a * y", VmValue::Int(12));
    test!("type X = ();               let X{ } = X{ }",                            VmValue::Void);
}

#[test] fn tuple_type_coercion() {
    test!("const typ = (bool, bool);  let x: typ = (true, false);  x^", VmValue::Tup(vec![VmValue::Bool(true), VmValue::Bool(false)]));
    test!("const typ = (bool; 2);     let x: typ = (true, false);  x^", VmValue::Tup(vec![VmValue::Bool(true), VmValue::Bool(false)]));
    test!("const typ = (bool, bool);  let x: typ = (true; 2);      x^", VmValue::Tup(vec![VmValue::Bool(true), VmValue::Bool(true)]));
    test!("const typ = (bool; 2);     let x: typ = (true; 2);      x^", VmValue::Tup(vec![VmValue::Bool(true), VmValue::Bool(true)]));

    test!("type X = (bool; 5);  X{ true; 5 }.3", VmValue::Bool(true));
}

#[test] fn consts() {
    test!("const (x, _) = (5, 3); x^", VmValue::Int(5));
    test!("const X = 2 * Y; const Z = 20; const Y = 3 * Z; X^", VmValue::Int(120));
}

#[test] fn impls() {
    test!("type Number = int; Number{ 320 }", VmValue::Int(320));

    test!("
        type N = int
        impl N {
            fn get() -> N => N{ 5 }
        }
        fn get() -> N => N{ 3 }

        (N.get(), get())
    ", VmValue::Tup(vec![VmValue::Int(5), VmValue::Int(3)]));

    test!("
        type N = int
        impl N {
            fn square(self: Self) -> Self {
                self * self
            }
            fn triangle(self: Self) -> Self {
                (self * (self + N{ 1 })) / N{ 2 }
            }
        }

        N{ 2 }.triangle().square().triangle().square()
    ", VmValue::Int(2025));
}

#[test] fn impls_out_of_order_resolution() {
    test!("
        impl A {
            const ONE: int = Self.double(B.TWO)
            const THREE: int = Self.double(1)
            fn double(n: int) -> int => n * 2
        }
        impl B {
            const TWO: int = Self.triple(A.THREE)
            fn triple(n: int) -> int => n * 3
        }
        type A = int;
        type B = int;

        A.ONE^
    ", VmValue::Int(12));
}

#[test] fn point_impl_test() {
    test!("
        type Point = (x: int, y: int)
        impl Point {
            fn squared_distance(self: &Self) -> int {
                self.x * self.x + self.y * self.y
            }
        }

        let mut p = Point{ x: 3, y: 2 }
        p.y = 4
        p.squared_distance()
    ", VmValue::Int(25));
}


#[test] fn enums() {
    test!("
        const Dir = enum { Up, Down{ bool, bool } }

        let down: Dir = :Down{ true, false }
        let down2: Dir = :Down{ true, false }
        let up: Dir = :Up
        let up2 = Dir.Up

        up^ is :Up
        and down^ is :Down{ _, false }
        and up2^ is !:Down{ _, _ }
        and down2^ is !:Up
    ", VmValue::Bool(true));

    test!("
        const Res = enum { Err, Ok{ (int, bool) } }
        let r: Res = :Ok{ (100, true) }

        r^ is :Ok{ (100, !false) }
    ", VmValue::Bool(true));
}

#[test] fn enum_refinement() {
    test!("
        fn handle_some(some: Option.Some) -> int {
            let :Some{ inner } = some^
            inner
        }

        let x: Option.Some = :Some{ 1 }
        handle_some(x^) + handle_some(:Some{ 2 })
    ", VmValue::Int(3));

    test!("
        let :Some{ inner } = Option.Some{ 3 }
        inner^
    ", VmValue::Int(3));
}

#[test] fn enum_impl_test() {
    test!("
        impl Option {
            fn is_some(self: Self) -> bool {
                self^ is :Some{ _ }
            }
            fn is_none(self: Self) -> bool {
                self^ is :None
            }
        }

        Option.Some{ 3 } is :Some{ _ }
        and Option.Some{ 3 }.is_some()
        and Option.is_some(:Some{ 3 })
        and !Option.Some{ 3 }.is_none()
        and !Option.None.is_some()
        and Option.None.is_none()
    ", VmValue::Bool(true));
}


#[test] fn prelude() {
    test!("Range{ start: 1, end: 2 }", VmValue::Tup(vec![VmValue::Int(2), VmValue::Int(1)]));
}
#[test] fn prelude_math_algorithms() {
    test!("0.sqrt()", VmValue::Int(0));
    test!("1.sqrt()", VmValue::Int(1));
    test!("168.sqrt()", VmValue::Int(12));
    test!("169.sqrt()", VmValue::Int(13));
    test!("170.sqrt()", VmValue::Int(13));

    test!("90.gcd(24)", VmValue::Int(6));
    test!("4.lcm(6)", VmValue::Int(12));
    test!("0.factorial()", VmValue::Int(1));
    test!("5.factorial()", VmValue::Int(120));
    test!("(-15).abs() + 15.abs()", VmValue::Int(30));

    test!("0.0.sqrt()", VmValue::Float(0.0));
    test!("4.0.sqrt()", VmValue::Float(2.0));
    test!("3.0.hypot(4.0)", VmValue::Float(5.0));
}

#[test] fn prelude_ranges_and_options() {
    test!("(5..15).contains(5)", VmValue::Bool(true));
    test!("(5..15).contains(15)", VmValue::Bool(false));
    test!("(5..=15).contains(15)", VmValue::Bool(true));
    test!("(10..5).len()", VmValue::Int(0));
    test!("(5..10).len()", VmValue::Int(5));

    test!("Option.None.unwrap_or(42)", VmValue::Int(42));
    test!("Option.Some{ 7 }.unwrap_or(42)", VmValue::Int(7));
}

#[test] fn prelude_contract_panics() {
    test_err!("Option.None.unwrap()", ErrType::RuntimeError { .. });
    test_err!("(-1).sqrt()", ErrType::RuntimeError { .. });
    test_err!("(-1.0).sqrt()", ErrType::RuntimeError { .. });
    test_err!("(-1).factorial()", ErrType::RuntimeError { .. });
}


#[test] fn coerce_branch_types() {
    test!(r#"  if "bla" is let "{x}" => x else ""  "#,        VmValue::OwnedStr("bla".to_string()));
    test!(r#"  match "bla" is let "{x}" => x, is _ => ""  "#, VmValue::OwnedStr("bla".to_string()));
    test!("
        let arr = (1, 2, 3, 4, 5)
        let s: &[int] = arr
        let coerce1 = if true  => arr else s
        let coerce2 = if false => arr else s
        let coerce3 = if false => s else arr
        let coerce4 = if true  => s else arr
        coerce1[1] + coerce2[2] + coerce3[3] + coerce4[4]
    ", VmValue::Int(14));

    test!("
        let x = (1, 2)
        let y = (3, 4, 5, 6, 7)
        let s1: &[int] = if false => x else y
        let s2: &[int] = if false => y else x
        s1.len() - s2.len()
    ", VmValue::Int(3));

    test!(r#"
        let s = "hello"
        let s = s
        let res = match 1
            is 0 => panic("zero")
            is 1 => s
            is _ => "other"
        res
    "#, VmValue::OwnedStr("hello".to_string()));

    test!(r#"
        let s = "yes"
        { #block
            break #block s
            "nop"
        }
    "#, VmValue::OwnedStr("yes".to_string()));
    test!(r#"
        let s = "nop"
        { #block
            break #block "yes"
            s
        }
    "#, VmValue::OwnedStr("yes".to_string()));
}


#[test] fn random_one_liners() {
    test!("fn triangle(n: int) -> int { let mut sum = 0; let mut i = 0; while i < n { i += 1; sum += i }; sum }; triangle(10)", VmValue::Int(55));
    test!("fn triangle(n: int) -> int { let mut res = 0; for i in 0..=n => res += i; res }; triangle(10)", VmValue::Int(55));
    test!("fn triangle(n: int) -> int => if n <= 1 => n else n + triangle(n - 1); triangle(10)", VmValue::Int(55));
    test!("fn triangle(n: int) -> int => n * (n + 1) / 2; triangle(10)", VmValue::Int(55));

    test!("fn fact(n: int) -> int { let mut res = 1; for i in 1..=n => res *= i; res }; fact(5)", VmValue::Int(120));
    test!("fn fact(n: int) -> int => if n <= 1 => n else n * fact(n - 1); fact(5)", VmValue::Int(120));

    test!("fn rev(mut n: int) -> int { let mut rev = 0; while n > 0 { rev = rev * 10 + (n % 10); n /= 10 }; rev }; rev(42069)", VmValue::Int(96024));

    test!(r#"
        if "https://youtu.be/dQw4w9WgXcQ" is let "{proto}://{domain}/{path}" => (proto, domain, path) else ("", "", "")
    "#, VmValue::Tup(vec![VmValue::OwnedStr("https".to_string()), VmValue::OwnedStr("youtu.be".to_string()), VmValue::OwnedStr("dQw4w9WgXcQ".to_string())]));

    test!("
        fn is_prime(n: int) -> bool {
            for d in 2..n {
                if n / d < d => break  // optimization
                if n % d is 0 => return false
            }
            n >= 2
        }
        let mut count = 0
        for i in 0..50 => if is_prime(i) => count += 1
        count
    ", VmValue::Int(15));
}