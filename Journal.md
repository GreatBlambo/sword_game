## 10/12

Progress on render passes. Made the node dependencies explicit, removed the "sort order" concept and modified it to allow as much overlap between dependent passes as possible.

From here, need to work on mapping to physical attachments...I went down the pass dependency rabbit hole today because I needed a structure which will give explicit dependencies between passes, rather than a sort order.