-- Lazy lists and recursion, in plain Haskell.
module Lists where

primes :: [Int]
primes = sieve [2..]
  where
    sieve [] = []
    sieve (p:xs) = p : sieve [x | x <- xs, x `mod` p /= 0]

fibs :: [Integer]
fibs = 0 : 1 : zipWith (+) fibs (tail fibs)

collatz :: Int -> [Int]
collatz 1 = [1]
collatz n
  | even n = n : collatz (n `div` 2)
  | otherwise = n : collatz (3 * n + 1)

sumTo :: Int -> Int
sumTo n = go n 0
  where
    go 0 acc = acc
    go k acc = go (k - 1) (acc + k)

quicksort :: [Int] -> [Int]
quicksort [] = []
quicksort (p:xs) = quicksort [x | x <- xs, x < p] ++ [p] ++ quicksort [x | x <- xs, x >= p]
