# Payments Engine

## Specification clarification
The specification for the transaction types is incomplete and there is room for interpretation. Here is my understanding that matches the described behaviour as closely as possible.

- Only deposit transactions can be disputed and reversed. Since a dispute decrements user's available funds, it would not make sense to dispute withdrawal transactions.
- When a dispute is processed, user's available funds may become negative (if the user has already spent the disputed funds). We could add a check to make sure user has enough available funds to issue a dispute, but I'm not sure this adds any value since the user can still reverse a deposit with the help of their bank. Note that my solution ignores any chargebacks without a prior dispute, but in reality a chargeback may occur even without the user issuing a dispute.
- A resolve transaction is essentially a cancellation of a previous dispute transaction, since the user's available and held funds are back to what they were prior to the dispute.
- After a chargeback is processed, user may end up with negative available funds (if they spent the disputed funds before the chargeback). Locking the account does not help to prevent this, however it helps to prevent any further malicious activity by the user.
- The engine will not process any transactions after an account is locked.

## Correctness and error handling
I have include some FV tests (inside `engine.rs`) that test the engine behaviour on different inputs and make sure it conforms to the spec. Malformed input entries are ignored with an error printed to stderr. I have also defined a custom error type for logical errors that may occur during execution. These are printed to stderr and the corresponding erroneous transactions are simply ignored. I have used `Result::expect` to unwrap in places where it is safe to do so.

## Storing currency values as Decimal types
I have used the `Decimal` type from the crate `rust_decimal` to store currency values. This ensures there are no rounding errors that may otherwise arise when representing certain decimal values as floating point binary numbers (e.g. 0.1 cannot be represented exactly as a float since it's not a sum of powers of 2). Such inaccuracies could result in transactions not being processed correctly (e.g. in the example `deposits_and_withdrawals.csv`, the withdrawal for client 1 would be rejected since the two deposits add up to slightly less than what they should when using floats).

Rounding is not required - inputs are assumed to be accurate to 4 decimal places and since we only ever perform addition and subtraction on the inputs, the accuracy is preserved in outputs.

## Efficiency and concurrency considerations
The `csv` crate is efficient at reading large files since it buffers its reads (instead of pre-loading the entire file into memory). Note, however, that the engine still stores transactions in memory inside a `HashMap`. In a real system, we could use a database to store data more efficiently on disk.

The current solution does not allow for concurrency - records are read from a single file and must be processed in chronological order. Alternativelly, the records could be streamed from many concurrent TCP connections. In this case, we could use an `mpsc` channel, where each worker handling a connection would send transactions onto the channel and a single dedicated worker would receive and process them in the order they were sent (assuming this is how tx ordering is determined for simplicity). 

We could even go a step further and parallelise the processing of transactions (e.g. if the processing is complex and becomes a bottleneck). Instead of a single dedicated worker for processing transactions, we could have many workers running in parallel, but we would somehow need to synchronize their access to the transactions to make sure two workers don't access the same data at the same time. One way to do this would be using a lock to guard access to the database where the transactions are stored. This may not be ideal and the lock could become higly contended resulting in lower performance. Another solution could sort user accounts into multiple buckets, shard the database accordingly and assign a worker per bucket. This way, any two workers operate on independent data that can be processed in parallel (this simple approach would not work if we need to accomodate for transactions between user accounts).
