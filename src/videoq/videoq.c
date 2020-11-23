#include <pthread.h>
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

/* Constants */
#define MAX_SEGMENTS 16

/* Flags */
#define FLAG_CONFLATION 1
#define FLAG_NO_SENDER 2

/* Error code */
#define NO_RECEIVERS 1
#define SENDER_CLOSED 2
#define MAX_RECEIVERS 3

/* Data structures */
struct Inner {
	/* Single mutex to move readers and writers between segments */
	/* An important part of our design is that memory segments can *
	 * be borrowed out with this mutex unlocked */
	pthread_mutex_t lock;

	/* Pointers to memory locations, these are borrowed out *
	 * independently in rust */
	uint8_t* segments[MAX_SEGMENTS];
	uint64_t timestamps[MAX_SEGMENTS];

	/* The numbers of borrows to each memory segment */
	uint8_t num_borrows[MAX_SEGMENTS];

	/* Total number of segments, we start with 2 *
	 * but can increase to MAX_SEGMENTS */
	size_t num_segments;

	/* Size in bytes of each segment */
	size_t bufsize;

	/* The index of the last block written by the writer */
	size_t last_written_block;
	/* The last but one block written by the writer *
	 * NOTE: If last_written_block isn't available then *
	 * this block must be */
	size_t prev_written_block;

	/* The number of receivers currently active */
	uint8_t num_receivers;

	/* May be set with the FLAG_* constants above */
	uint8_t flags;
};

typedef struct Inner* RingQ;

struct Sender {
	RingQ ringq;
	size_t bufsize;
};

struct Receiver {
	RingQ ringq;
	size_t bufsize;
	size_t index;
	uint8_t* data_ptr;
	uint64_t timestamp;
};

struct SenderReceiverPair {
	struct Sender sender;
	struct Receiver receiver;
};


/* Functions definitions */
void init_ringq(RingQ);
size_t _get_free_writer(RingQ);
void free_ringq(RingQ);
size_t _get_recv_index(RingQ);
int _new_segment(RingQ);

/* public functions */
struct SenderReceiverPair
new_ringq(size_t bufsize) {
	struct Sender sender;
	struct Receiver receiver;
	struct SenderReceiverPair pair;
	RingQ ringq;

	ringq = (RingQ) malloc(sizeof(struct Inner));
	ringq->bufsize = bufsize;
	init_ringq(ringq);

	sender.ringq = ringq;
	sender.bufsize = bufsize;
	receiver.ringq = ringq;
	receiver.bufsize = bufsize;

	pair.sender = sender;
	pair.receiver = receiver;
	return pair;
}

int
send(struct Sender* sender, uint8_t* data, uint64_t timestamp) {
	RingQ ringq;
	ringq = sender->ringq;
	size_t free_writer;

    pthread_mutex_lock(&(ringq->lock));

    /* There are no receivers *
     * return this */
    if (ringq->num_receivers == 0) {
    	return NO_RECEIVERS;
    }

    /* Get a currently unused SEGMENT *
     * there is guaranteed to be one */
    free_writer = _get_free_writer(ringq);

    if (free_writer == ringq->last_written_block) {
    	/* If we're writing the same SEGMENT as the *
    	 * previous write, set the conflation flag */
    	ringq->flags |= FLAG_CONFLATION;
    } else {
    	ringq->prev_written_block = ringq->last_written_block;
    }

    pthread_mutex_unlock(&(ringq->lock));

    /* We don't need the mutex, why?
     *   - existing readers are reading from a different index
     *     to the free_writer index
     *   - new readers will take either the last_written_block
     *     or prev_written block index to borrow video frames.
     */

	memcpy(ringq->segments[free_writer], data, ringq->bufsize);
	ringq->timestamps[free_writer] = timestamp;

    pthread_mutex_lock(&(ringq->lock));
	ringq->last_written_block = free_writer;
	ringq->flags &= ~FLAG_CONFLATION;
    pthread_mutex_unlock(&(ringq->lock));

    return 0;
}

void
free_sender(struct Sender* sender) {
	RingQ ringq;
	ringq = sender->ringq;

    pthread_mutex_lock(&(ringq->lock));
    ringq->flags |= FLAG_NO_SENDER;
    if (ringq->num_receivers == 0) {
    	/* If all receivers have already been dropped *
    	 * then we need to free the memory allocated by the *
    	 * RingQ */
    	free_ringq(ringq);
    } else {
    	/* We only unlock the mutex if we won't free *
    	 * this is because we could have free'd the mutex */
	    pthread_mutex_unlock(&(ringq->lock));
    }
}

int
start_recv(struct Receiver* receiver) {
	size_t index;
	int ret;
	ret = 0;

	RingQ ringq;
	ringq = receiver->ringq;

    pthread_mutex_lock(&(ringq->lock));
    if (ringq->flags & FLAG_NO_SENDER) {
    	ret = SENDER_CLOSED;
    } else {
	    index = _get_recv_index(ringq);
	    ringq->num_borrows[index]++;
    }
    pthread_mutex_unlock(&(ringq->lock));

    if (ret != 0)
    	return ret;

    receiver->index = index;
    receiver->data_ptr = ringq->segments[index];
    receiver->timestamp = ringq->timestamps[index];
    return 0;
}

int
end_recv(struct Receiver* receiver) {
	RingQ ringq;
	ringq = receiver->ringq;

    pthread_mutex_lock(&(ringq->lock));
	ringq->num_borrows[receiver->index]--;
    pthread_mutex_unlock(&(ringq->lock));

    return 0;
}

struct Receiver
new_receiver(struct Receiver* receiver, int* error) {
	RingQ ringq;
	struct Receiver new_receiver;

	ringq = receiver->ringq;

	new_receiver.ringq = ringq;
	new_receiver.bufsize = receiver->bufsize;

    pthread_mutex_lock(&(ringq->lock));
    *error = _new_segment(ringq);
    if (*error == 0) {
	    ringq->num_receivers++;
    }
    pthread_mutex_unlock(&(ringq->lock));

	return new_receiver;
}

void
init_ringq(RingQ ringq) {
	int i;

	for (i = 0; i < MAX_SEGMENTS; i++) {
		ringq->segments[i] = NULL;
		ringq->num_borrows[i] = 0;
		ringq->timestamps[i] = 0;
	}

	/* Allocate some memory */
	ringq->segments[0] = (uint8_t*) malloc(ringq->bufsize);
	ringq->segments[1] = (uint8_t*) malloc(ringq->bufsize);
	ringq->segments[2] = (uint8_t*) malloc(ringq->bufsize);
	ringq->num_segments = 3;

	/* No written blocks yet */
	ringq->last_written_block = 0;
	ringq->prev_written_block = 1;
	ringq->num_receivers = 1;
	ringq->flags = 0;

	pthread_mutex_init(&(ringq->lock), NULL);
}

/* _get_free_writer returns an index for which *
 * there are currently no readers borrowing the *
 * data in said index. NOTE: The mutex MUST be locked *
 * above this function */
size_t
_get_free_writer(RingQ ringq) {
	size_t free_writer;
	size_t i;

	free_writer = ringq->last_written_block;

	for (i = 0; i < ringq->num_segments; i++) {
		/* We avoid the last_written_block if possible *
		 * this allows last_written_block to be used by *
		 * the next receiver */
		if (i == ringq->last_written_block)
			continue;

		/* If this block has no readers borrowing the *
		 * data then we can write to it */
		if (ringq->num_borrows[i] == 0) {
			free_writer = i;
			break;
		}
	}

	return free_writer;
}

/* Free any memory allocated by the Ringq *
 * NOTE: This must be called only when there *
 * is only active sender OR receiver. It's not *
 * threadsafe */
void
free_ringq(RingQ ringq) {
	size_t i;

	/* Free any active segments */
	for (i = 0; i < ringq->num_segments; i++)
		free(ringq->segments[i]);

	pthread_mutex_destroy(&(ringq->lock));
	free(ringq);
}

/* return the index for a new reader *
 * this functions requires a mutex lock *
 * above */
size_t
_get_recv_index(RingQ ringq) {
	if (ringq->flags & FLAG_CONFLATION) {
		return ringq->last_written_block;
	} else {
		return ringq->prev_written_block;
	}
}

/* Create a new memory segment, this is done *
 * when a receiver is cloned. The mutex must be *
 * locked above this function */
int
_new_segment(RingQ ringq) {
	if (ringq->num_segments == MAX_SEGMENTS)
		return MAX_RECEIVERS;

	uint8_t* segment;
	segment = (uint8_t*) malloc(ringq->bufsize);
	ringq->segments[ringq->num_segments++] = segment;

	return 0;
}
