struct Worker;
impl Worker {
    fn measure(&self) {}
}
fn run(w: Worker) { w.measure(); }
