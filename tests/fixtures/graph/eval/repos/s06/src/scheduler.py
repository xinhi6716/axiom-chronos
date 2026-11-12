def order_jobs(jobs):
    return sorted(jobs, key=lambda job: job["priority"])
