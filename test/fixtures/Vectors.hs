-- A module with an export list, an infix constructor and a user operator,
-- imported by the tier-1 tests.
module Vectors (Vec(..), (<+>), scale, norm, hidden) where

infixl 6 <+>
infixr 5 :|

data Vec = Double :| Double deriving (Eq, Show)

(<+>) :: Vec -> Vec -> Vec
(a :| b) <+> (c :| d) = (a + c) :| (b + d)

scale :: Double -> Vec -> Vec
scale k (a :| b) = k * a :| k * b

norm :: Vec -> Double
norm (a :| b) = sqrt (a * a + b * b)

hidden :: Int
hidden = 42

secret :: Int
secret = 7
